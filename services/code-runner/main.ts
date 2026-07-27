// Sandboxed code runner for NodeFlare code-execution mode.
//
// Runs as its own Fly app (= Firecracker microVM), separate from the proxy. It accepts
// AI-written JavaScript and executes each request in a FRESH, locked-down Deno
// subprocess:
//   deno run --allow-net=<tools-endpoint-host> --no-prompt -
// i.e. no file system, no env, no subprocess, and network restricted to ONLY the
// proxy's scope-enforced tool-call endpoint. The injected `tools.*` API POSTs there
// with the per-execution token; that endpoint — not this wrapper — is the security
// boundary (it re-checks scope server-side). Even if user code bypasses `tools` and
// calls fetch() directly, --allow-net limits it to that one host and the token only
// grants the execution's own scope.
//
// This service itself is trusted code; it runs with --allow-net (to serve) and
// --allow-run=deno (to spawn the sandbox).
//
// SECURITY: `/run` executes caller-supplied code AND lets the caller pick the sandbox's
// only network-egress host (via `tools_endpoint`). It must therefore only ever be called
// by the proxy. When `CODE_RUNNER_TOKEN` is set, every `/run` request must present it as
// `Authorization: Bearer <token>`; the proxy sends the same value. Set it on both apps
// (`fly secrets set CODE_RUNNER_TOKEN=... -a mcp-code-runner` and `-a mcp-cloud-proxy`).

const SERVICE_PORT = Number(Deno.env.get("PORT") ?? "8080");

// DoS guards for each sandbox (the VM is only ~1GB and runs many concurrently):
//  - V8 old-space cap turns runaway allocation into a self-kill of that one sandbox
//    instead of OOM-ing the whole runner VM.
//  - Output cap bounds how much we buffer from a flooding sandbox.
const MAX_HEAP_MB = Number(Deno.env.get("RUNNER_MAX_HEAP_MB") ?? "128");
const MAX_OUTPUT_BYTES = Number(Deno.env.get("RUNNER_MAX_OUTPUT_BYTES") ?? String(4 * 1024 * 1024));

// Shared secret required from the proxy. Fail-closed: refuse to start without it.
const RUN_TOKEN = Deno.env.get("CODE_RUNNER_TOKEN") ?? "";
if (!RUN_TOKEN) {
  console.error(
    "[startup] FATAL: CODE_RUNNER_TOKEN is unset. " +
      "Set the same secret on this app and the proxy. Refusing to start.",
  );
  Deno.exit(1);
}

// Constant-time string comparison to avoid leaking the token via response timing.
function safeEqual(a: string, b: string): boolean {
  const ea = new TextEncoder().encode(a);
  const eb = new TextEncoder().encode(b);
  if (ea.length !== eb.length) return false;
  let diff = 0;
  for (let i = 0; i < ea.length; i++) diff |= ea[i] ^ eb[i];
  return diff === 0;
}

interface RunRequest {
  code: string;
  token: string;
  tools_endpoint: string;
  timeout_secs?: number;
  max_tool_calls?: number;
}

function bootstrap(req: RunRequest, guid: string): string {
  const maxCalls = req.max_tool_calls ?? 50;
  // Values are JSON-encoded so embedding them as source is safe. The user code is
  // appended as-is and executed inside an async IIFE; the sandbox (not escaping) is
  // the security boundary, so we don't need to sanitize it.
  return `
const __TOKEN = ${JSON.stringify(req.token)};
const __ENDPOINT = ${JSON.stringify(req.tools_endpoint)};
const __MAX = ${maxCalls};
const __GUID = ${JSON.stringify(guid)};
// Route user console output to stderr so stdout carries only our result frame.
console.log = (...a) => console.error(...a);
console.info = (...a) => console.error(...a);
console.debug = (...a) => console.error(...a);
let __calls = 0;
const tools = new Proxy({}, {
  get(_t, name) {
    return async (args) => {
      if (typeof name !== "string") throw new Error("invalid tool name");
      if (++__calls > __MAX) throw new Error("tool call limit (" + __MAX + ") exceeded");
      const res = await fetch(__ENDPOINT, {
        method: "POST",
        headers: { "content-type": "application/json", "authorization": "Bearer " + __TOKEN },
        body: JSON.stringify({ tool: name, arguments: args ?? {} }),
      });
      if (!res.ok) throw new Error("tool '" + name + "' failed: HTTP " + res.status);
      const j = await res.json();
      // The proxy returns tool-level errors as { __toolError } so we can raise them here
      // as normal exceptions the user code can try/catch (instead of an opaque HTTP 502).
      if (j && typeof j === "object" && "__toolError" in j) {
        const m = typeof j.__toolError === "string" ? j.__toolError : JSON.stringify(j.__toolError);
        throw new Error("tool '" + name + "' error: " + m);
      }
      return j;
    };
  },
});
const __emit = async (marker, payload) => {
  // Deno.stdout.write may write partially; loop until the whole frame is out so the
  // result is never truncated (or dropped) before the process exits.
  const bytes = new TextEncoder().encode("\\n" + __GUID + marker + "\\n" + JSON.stringify(payload ?? null));
  let n = 0;
  while (n < bytes.length) n += await Deno.stdout.write(bytes.subarray(n));
};
try {
  const __result = await (async () => { ${req.code}
  })();
  console.error("[sandbox] result type=" + typeof __result);
  await __emit(":OK:", __result ?? null);
} catch (e) {
  console.error("[sandbox] threw: " + String((e && e.stack) || e));
  await __emit(":ERR:", String((e && e.message) || e));
}
`;
}

// Drain a stream, but stop (and report) once it exceeds `maxBytes` so a flooding sandbox
// can't grow this parent process's memory without bound.
async function readCapped(
  stream: ReadableStream<Uint8Array>,
  maxBytes: number,
): Promise<{ data: Uint8Array; truncated: boolean }> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  let truncated = false;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value) continue;
      chunks.push(value);
      total += value.length;
      if (total > maxBytes) {
        truncated = true;
        break;
      }
    }
  } finally {
    try {
      reader.releaseLock();
    } catch { /* ignore */ }
  }
  const data = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    data.set(c, off);
    off += c.length;
  }
  return { data, truncated };
}

async function runSandboxed(req: RunRequest): Promise<{ output?: unknown; error?: string }> {
  const t0 = Date.now();
  let host: string;
  try {
    host = new URL(req.tools_endpoint).host;
  } catch {
    console.error(`[run] invalid tools_endpoint: ${req.tools_endpoint}`);
    return { error: "invalid tools_endpoint" };
  }
  // Prevent --allow-net from accidentally receiving multiple hosts if the
  // endpoint URL is crafted to contain commas, newlines, or spaces.
  if (/[,\n\r ]/.test(host)) {
    console.error(`[run] rejected: tools_endpoint host contains disallowed characters: ${JSON.stringify(host)}`);
    return { error: "invalid tools_endpoint: host contains disallowed characters" };
  }
  const guid = crypto.randomUUID();
  const program = bootstrap(req, guid);
  const timeoutMs = (req.timeout_secs ?? 15) * 1000;
  console.error(
    `[run] start: code=${req.code.length}B allow-net=${host} timeout=${timeoutMs}ms max_calls=${req.max_tool_calls ?? 50}`,
  );

  const command = new Deno.Command("deno", {
    // --v8-flags caps the sandbox's JS heap so runaway allocation self-kills instead of
    // taking down the runner VM.
    args: [
      "run",
      "--no-prompt",
      `--v8-flags=--max-old-space-size=${MAX_HEAP_MB}`,
      `--allow-net=${host}`,
      "-",
    ],
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  });
  const child = command.spawn();

  // Feed the program via stdin (no temp files, no fs permission needed).
  const writer = child.stdin.getWriter();
  await writer.write(new TextEncoder().encode(program));
  await writer.close();

  // Enforce a wall-clock timeout: kill the subprocess tree if it overruns.
  const timer = setTimeout(() => {
    try {
      child.kill("SIGKILL");
    } catch { /* already exited */ }
  }, timeoutMs);

  // Read stdout/stderr with a hard byte cap (DoS guard). If either floods past the cap we
  // kill the sandbox rather than buffer it all in this parent process.
  const [outR, errR] = await Promise.all([
    readCapped(child.stdout, MAX_OUTPUT_BYTES),
    readCapped(child.stderr, MAX_OUTPUT_BYTES),
  ]);
  if (outR.truncated || errR.truncated) {
    try {
      child.kill("SIGKILL");
    } catch { /* already exited */ }
  }
  const status = await child.status;
  clearTimeout(timer);
  const exitCode = status.code;
  const stdout = outR.data;
  const stderr = errR.data;
  const ms = Date.now() - t0;

  if (outR.truncated) {
    console.error(`[run] rejected: stdout exceeded ${MAX_OUTPUT_BYTES}B cap in ${ms}ms`);
    return { error: `output exceeded ${MAX_OUTPUT_BYTES}-byte limit` };
  }

  // Sandbox logs (user console.* + tool-call errors) land on stderr.
  const errText = new TextDecoder().decode(stderr).trim();
  if (errText) {
    console.error(`[run] sandbox stderr:\n${errText.slice(0, 1200)}`);
  }

  const text = new TextDecoder().decode(stdout);
  // Raw stdout dump (truncated) so we can see exactly what the sandbox emitted.
  const rawPreview = text.length > 700
    ? `${text.slice(0, 400)} …[total ${text.length}B]… ${text.slice(-200)}`
    : text;
  console.error(`[run] raw stdout (${text.length}B): ${JSON.stringify(rawPreview)}`);
  const okMarker = "\n" + guid + ":OK:\n";
  const errMarker = "\n" + guid + ":ERR:\n";

  const okIdx = text.lastIndexOf(okMarker);
  if (okIdx !== -1) {
    const json = text.slice(okIdx + okMarker.length);
    console.error(`[run] done OK in ${ms}ms (result=${json.length}B)`);
    try {
      return { output: JSON.parse(json) };
    } catch {
      return { output: json };
    }
  }
  const errIdx = text.lastIndexOf(errMarker);
  if (errIdx !== -1) {
    const json = text.slice(errIdx + errMarker.length);
    console.error(`[run] done ERR in ${ms}ms: ${json}`);
    try {
      return { error: String(JSON.parse(json)) };
    } catch {
      return { error: json };
    }
  }
  console.error(`[run] no result in ${ms}ms (exit=${exitCode}, timed out or crashed)`);
  const hint = errText ? `\n${errText.slice(0, 500)}` : "";
  return { error: `execution produced no result (timed out or crashed)${hint}` };
}

// Bind IPv6 (dual-stack): Fly private networking (`.internal`) is IPv6-only, so the
// proxy reaches us over 6PN. `0.0.0.0` (IPv4) would refuse those connections.
Deno.serve({ port: SERVICE_PORT, hostname: "::" }, async (req) => {
  const url = new URL(req.url);
  if (req.method === "GET" && url.pathname === "/health") {
    return new Response("ok");
  }
  if (req.method === "POST" && url.pathname === "/run") {
    console.error("[run] request received");
    // Always require the shared secret (RUN_TOKEN is guaranteed non-empty at startup).
    if (!safeEqual(req.headers.get("authorization") ?? "", `Bearer ${RUN_TOKEN}`)) {
      console.error("[run] rejected: missing/invalid runner token");
      return Response.json({ error: "unauthorized" }, { status: 401 });
    }
    let body: RunRequest;
    try {
      body = await req.json();
    } catch {
      console.error("[run] rejected: invalid JSON body");
      return Response.json({ error: "invalid JSON body" }, { status: 400 });
    }
    if (!body.code || !body.token || !body.tools_endpoint) {
      console.error("[run] rejected: missing code/token/tools_endpoint");
      return Response.json({ error: "missing code/token/tools_endpoint" }, { status: 400 });
    }
    const result = await runSandboxed(body);
    return Response.json(result);
  }
  return new Response("not found", { status: 404 });
});
