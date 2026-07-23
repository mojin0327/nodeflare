-- Fix run_code tool description and schema to explicitly require JavaScript/TypeScript.
--
-- The previous description mentioned "python" as an example language, which caused
-- LLMs to generate Python code. The Deno sandbox only supports JavaScript/TypeScript.
-- Removing the `language` field entirely since the proxy ignores it — it only extracts
-- the `code` field and executes it as JavaScript in Deno.
UPDATE proxy_meta_tools
SET
    description  = 'Execute JavaScript/TypeScript code in a secure Deno sandbox. The sandbox exposes an injected `tools` object — call `await tools.<toolName>(args)` to invoke any server tool. Return a value from your code to get the result back. IMPORTANT: Only JavaScript/TypeScript is supported. Do NOT write Python or any other language.',
    input_schema = '{"type":"object","properties":{"code":{"type":"string","description":"JavaScript or TypeScript code to execute. Use `await tools.<name>(args)` to call server tools. Return a value to produce output."}},"required":["code"]}'::jsonb
WHERE handler_type = 'run_code';
