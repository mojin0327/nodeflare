use anyhow::{Context, Result};
use std::net::Ipv4Addr;
use tokio::process::Command;

/// A TAP device used to connect a Firecracker VM to the host network.
///
/// IP layout per VM (derived deterministically from container_name):
///   /30 subnet: 172.16.A.{B, B+1, B+2, B+3}  where B is /30-aligned (B % 4 == 0)
///   host side:  172.16.A.B+1  (assigned to the tap interface on the host)
///   VM side:    172.16.A.B+2  (assigned to eth0 inside the guest)
///   gateway:    172.16.A.B+1  (same as host side — the host IS the router)
pub struct TapDevice {
    /// e.g. "tap-a3f9c1e2" (max 15 chars, kernel limit)
    pub name: String,
    pub host_ip: Ipv4Addr,
    pub vm_ip: Ipv4Addr,
}

impl TapDevice {
    /// Derive a deterministic TAP device from a container name.
    /// Uses FNV-1a hash to distribute across 172.16.0.0/16, avoiding .0 and .255.
    pub fn derive(container_name: &str) -> Self {
        let hash = container_name
            .bytes()
            .fold(2_166_136_261u32, |h, b| (h ^ b as u32).wrapping_mul(16_777_619));

        // Use bits 8..15 and 0..7 for the last two octets.
        // Align to /30 boundary (multiples of 4): host = base+1, vm = base+2.
        // This avoids assigning the broadcast address (.base+3) to the VM,
        // which happened when base was even but not /30-aligned (e.g. base=198 → vm=199=bcast).
        let second = ((hash >> 8) & 0xFF) as u8;
        let base = (hash & 0xFC) as u8; // /30-aligned (divisible by 4)

        let host_ip = Ipv4Addr::new(172, 16, second, base + 1);
        let vm_ip = Ipv4Addr::new(172, 16, second, base + 2);

        // Tap name: "fc-" + first 8 hex chars of hash (11 chars total, well under 15)
        let name = format!("fc-{:08x}", hash);

        Self { name, host_ip, vm_ip }
    }

    /// MAC address for the VM's virtual NIC (derived from hash, locally administered).
    pub fn vm_mac(&self) -> String {
        let octs = self.vm_ip.octets();
        format!("02:fc:{:02x}:{:02x}:{:02x}:01", octs[1], octs[2], octs[3])
    }

    /// Create the TAP device, assign IPs, and set up NAT so the VM can reach the internet.
    pub async fn create(&self) -> Result<()> {
        // Create TAP device
        run("ip", &["tuntap", "add", &self.name, "mode", "tap"]).await
            .with_context(|| format!("ip tuntap add {}", self.name))?;

        // Assign host-side IP
        let host_cidr = format!("{}/30", self.host_ip);
        run("ip", &["addr", "add", &host_cidr, "dev", &self.name]).await
            .with_context(|| format!("ip addr add {} dev {}", host_cidr, self.name))?;

        run("ip", &["link", "set", &self.name, "up"]).await
            .with_context(|| format!("ip link set {} up", self.name))?;

        // Allow VM traffic to reach the internet via host's default interface
        run("sh", &["-c", &format!(
            "iptables -t nat -C POSTROUTING -s {}/30 -j MASQUERADE 2>/dev/null || \
             iptables -t nat -A POSTROUTING -s {}/30 -j MASQUERADE",
            self.host_ip, self.host_ip
        )]).await.ok(); // non-fatal if iptables is not available

        // Enable IP forwarding (idempotent)
        run("sh", &["-c", "echo 1 > /proc/sys/net/ipv4/ip_forward"]).await.ok();

        Ok(())
    }

    /// Forward host port to VM port so the Proxy can reach the VM via 127.0.0.1.
    /// Uses OUTPUT chain DNAT because the connection originates on the host itself.
    ///
    /// Two rules are needed:
    ///   1. OUTPUT DNAT: redirect 127.0.0.1:{port} → {vm_ip}:{port}
    ///   2. POSTROUTING SNAT: rewrite source from 127.0.0.1 → {host_ip} on the TAP
    ///      outgoing interface, so the VM's kernel doesn't reject the packet as a
    ///      martian (loopback source arriving on a non-loopback interface).
    pub async fn add_port_forward(&self, port: u16) -> Result<()> {
        let vm_dest = format!("{}:{}", self.vm_ip, port);
        run("sh", &["-c", &format!(
            "iptables -t nat -C OUTPUT -d 127.0.0.1 -p tcp --dport {port} -j DNAT --to-destination {vm_dest} 2>/dev/null || \
             iptables -t nat -A OUTPUT -d 127.0.0.1 -p tcp --dport {port} -j DNAT --to-destination {vm_dest}",
            port = port, vm_dest = vm_dest
        )]).await.context("iptables DNAT rule (OUTPUT)")?;

        let tap = &self.name;
        let host_ip = &self.host_ip;
        run("sh", &["-c", &format!(
            "iptables -t nat -C POSTROUTING -o {tap} -s 127.0.0.1 -p tcp --dport {port} -j SNAT --to-source {host_ip} 2>/dev/null || \
             iptables -t nat -A POSTROUTING -o {tap} -s 127.0.0.1 -p tcp --dport {port} -j SNAT --to-source {host_ip}",
            tap = tap, port = port, host_ip = host_ip
        )]).await.context("iptables SNAT rule (POSTROUTING)")?;

        Ok(())
    }

    /// Remove the port forward rules.
    pub async fn remove_port_forward(&self, port: u16) -> Result<()> {
        let vm_dest = format!("{}:{}", self.vm_ip, port);
        run("sh", &["-c", &format!(
            "iptables -t nat -D OUTPUT -d 127.0.0.1 -p tcp --dport {port} -j DNAT --to-destination {vm_dest} 2>/dev/null || true",
            port = port, vm_dest = vm_dest
        )]).await.ok();

        let tap = &self.name;
        let host_ip = &self.host_ip;
        run("sh", &["-c", &format!(
            "iptables -t nat -D POSTROUTING -o {tap} -s 127.0.0.1 -p tcp --dport {port} -j SNAT --to-source {host_ip} 2>/dev/null || true",
            tap = tap, port = port, host_ip = host_ip
        )]).await.ok();

        Ok(())
    }

    /// Tear down the TAP device and remove NAT rules.
    pub async fn destroy(&self, port: u16) -> Result<()> {
        self.remove_port_forward(port).await.ok();

        run("ip", &["link", "del", &self.name]).await.ok();

        run("sh", &["-c", &format!(
            "iptables -t nat -D POSTROUTING -s {}/30 -j MASQUERADE 2>/dev/null || true",
            self.host_ip
        )]).await.ok();

        Ok(())
    }
}

/// Network namespace for a fork VM.
///
/// Firecracker snapshots store the TAP device name.  Restoring two VMs from the
/// same snapshot would require them to share the same TAP, which the kernel
/// rejects as EBUSY.  The solution is to run each fork Firecracker process in
/// its own Linux network namespace that contains a TAP with the *same name* as
/// the base VM's TAP.  A veth pair connects the namespace to the host so the
/// proxy can reach the fork VM.
///
/// Packet flow (proxy → fork VM):
///   127.0.0.1:FORK_PORT
///   → host mangle mark=FORK_PORT
///   → host nat DNAT: dst → BASE_VM_IP:FORK_PORT
///   → host policy route (fwmark): route BASE_VM_IP via veth_host
///   → host nat SNAT on veth_host: src → host_veth_ip
///   → veth_host → veth_ns (inside fork netns)
///   → netns routing: BASE_VM_IP → BASE_TAP (base host_ip/30)
///   → netns nat SNAT on BASE_TAP: src → BASE_HOST_IP
///   → Firecracker → VM eth0  (sees src=BASE_HOST_IP, dst=BASE_VM_IP) ✓
pub struct ForkNetns {
    /// Name of the Linux network namespace, e.g. "ns-a3f9c1e2"
    pub ns_name: String,
    /// Host-side veth interface, e.g. "v0-a3f9c1e2" (≤15 chars)
    pub veth_host: String,
    /// Namespace-side veth interface, e.g. "v1-a3f9c1e2"
    pub veth_ns: String,
    /// IP on the host-side veth (169.254.A.B+1/30)
    pub host_veth_ip: Ipv4Addr,
    /// IP on the namespace-side veth (169.254.A.B+2/30)
    pub ns_veth_ip: Ipv4Addr,
}

impl ForkNetns {
    pub fn derive(fork_container: &str) -> Self {
        let hash = fork_container
            .bytes()
            .fold(2_166_136_261u32, |h, b| (h ^ b as u32).wrapping_mul(16_777_619));
        let a = ((hash >> 8) & 0xFF) as u8;
        let b = (hash & 0xFC) as u8; // /30-aligned
        let short = format!("{:08x}", hash);
        Self {
            ns_name: format!("ns-{}", short),
            veth_host: format!("v0-{}", short),
            veth_ns: format!("v1-{}", short),
            host_veth_ip: Ipv4Addr::new(169, 254, a, b + 1),
            ns_veth_ip: Ipv4Addr::new(169, 254, a, b + 2),
        }
    }

    /// Create the fork network namespace, TAP, and veth pair.
    ///
    /// After this call Firecracker can be started with `ip netns exec ns_name firecracker …`
    /// and the snapshot/load API will succeed because the expected TAP name exists inside
    /// the namespace.
    pub async fn create(&self, base_tap: &TapDevice) -> Result<()> {
        // Create namespace (idempotent: ignore "already exists")
        run("ip", &["netns", "add", &self.ns_name]).await.ok();

        // Inside netns: create TAP with the same name the snapshot expects
        ns_run(&self.ns_name, "ip", &["tuntap", "add", &base_tap.name, "mode", "tap"]).await
            .context("fork netns: tuntap add")?;
        let tap_cidr = format!("{}/30", base_tap.host_ip);
        ns_run(&self.ns_name, "ip", &["addr", "add", &tap_cidr, "dev", &base_tap.name]).await
            .context("fork netns: assign TAP IP")?;
        ns_run(&self.ns_name, "ip", &["link", "set", &base_tap.name, "up"]).await
            .context("fork netns: TAP up")?;

        // Create veth pair on host then move one end into the namespace
        run("ip", &["link", "add", &self.veth_host, "type", "veth", "peer", "name", &self.veth_ns]).await
            .context("fork veth create")?;
        run("ip", &["link", "set", &self.veth_ns, "netns", &self.ns_name]).await
            .context("fork veth move to netns")?;

        // Assign IPs and bring up
        let hcidr = format!("{}/30", self.host_veth_ip);
        run("ip", &["addr", "add", &hcidr, "dev", &self.veth_host]).await
            .context("fork veth_host IP")?;
        run("ip", &["link", "set", &self.veth_host, "up"]).await
            .context("fork veth_host up")?;

        let ncidr = format!("{}/30", self.ns_veth_ip);
        ns_run(&self.ns_name, "ip", &["addr", "add", &ncidr, "dev", &self.veth_ns]).await
            .context("fork veth_ns IP")?;
        ns_run(&self.ns_name, "ip", &["link", "set", &self.veth_ns, "up"]).await
            .context("fork veth_ns up")?;

        // Inside netns: default route exits via veth_ns → host
        ns_run(&self.ns_name, "ip", &[
            "route", "add", "default",
            "via", &self.host_veth_ip.to_string(),
            "dev", &self.veth_ns,
        ]).await.context("fork netns default route")?;

        // Inside netns: enable forwarding
        ns_run(&self.ns_name, "sh", &["-c", "echo 1 > /proc/sys/net/ipv4/ip_forward"]).await.ok();

        // Inside netns: SNAT traffic leaving the TAP (→ VM) so the VM sees the
        // gateway address it expects (BASE_HOST_IP) regardless of where it came from.
        ns_run(&self.ns_name, "sh", &["-c", &format!(
            "iptables -t nat -A POSTROUTING -o {} -j SNAT --to-source {}",
            base_tap.name, base_tap.host_ip
        )]).await.context("fork netns SNAT")?;

        // Enable host IP forwarding (idempotent)
        run("sh", &["-c", "echo 1 > /proc/sys/net/ipv4/ip_forward"]).await.ok();

        Ok(())
    }

    /// Set up host-side iptables/routing so 127.0.0.1:fork_port reaches the fork VM.
    ///
    /// `base_port` is the port the VM is actually listening on (derived from the base
    /// container name); `fork_port` is the ephemeral host port the proxy connects to.
    /// DNAT rewrites dst port fork_port → base_port so the VM accepts the connection.
    pub async fn add_port_forward(
        &self,
        fork_port: u16,
        base_vm_ip: Ipv4Addr,
        base_port: u16,
    ) -> Result<()> {
        let table = fork_port as u32;

        // Mark packets destined for fork_port in mangle OUTPUT (before nat DNAT fires)
        run("sh", &["-c", &format!(
            "iptables -t mangle -C OUTPUT -d 127.0.0.1 -p tcp --dport {p} -j MARK --set-mark {p} 2>/dev/null || \
             iptables -t mangle -A OUTPUT -d 127.0.0.1 -p tcp --dport {p} -j MARK --set-mark {p}",
            p = fork_port
        )]).await.context("fork mangle mark")?;

        // Policy routing: fwmark fork_port → table fork_port
        run("sh", &["-c", &format!(
            "ip rule show | grep -qF 'fwmark 0x{p:x}' || ip rule add fwmark {p} table {t}",
            p = fork_port, t = table
        )]).await.context("fork ip rule")?;

        // In policy table: route BASE_VM_IP via veth_host
        run("sh", &["-c", &format!(
            "ip route replace {vm}/32 dev {veth} src {hvip} table {t}",
            vm = base_vm_ip, veth = self.veth_host,
            hvip = self.host_veth_ip, t = table
        )]).await.context("fork ip route")?;

        // Static ARP entry so the host can send packets to BASE_VM_IP through the veth
        // without an ARP timeout: the veth_ns MAC is looked up inside the namespace.
        let mac_out = Command::new("ip")
            .args(["-n", &self.ns_name, "link", "show", &self.veth_ns])
            .output()
            .await
            .context("fork ARP: ip link show veth_ns")?;
        let mac_str = String::from_utf8_lossy(&mac_out.stdout);
        let mac = mac_str
            .lines()
            .find(|l| l.trim_start().starts_with("link/ether"))
            .and_then(|l| l.split_whitespace().nth(1))
            .ok_or_else(|| anyhow::anyhow!("could not find veth_ns MAC for {}", self.veth_ns))?
            .to_string();
        run("ip", &["neigh", "replace", &base_vm_ip.to_string(),
            "lladdr", &mac, "dev", &self.veth_host]).await
            .context("fork static ARP")?;

        // DNAT: 127.0.0.1:fork_port → BASE_VM_IP:base_port
        // The VM listens on base_port (derived from the base container), not fork_port.
        let dest = format!("{}:{}", base_vm_ip, base_port);
        run("sh", &["-c", &format!(
            "iptables -t nat -C OUTPUT -d 127.0.0.1 -p tcp --dport {fp} -j DNAT --to-destination {d} 2>/dev/null || \
             iptables -t nat -A OUTPUT -d 127.0.0.1 -p tcp --dport {fp} -j DNAT --to-destination {d}",
            fp = fork_port, d = dest
        )]).await.context("fork DNAT")?;

        // SNAT on veth_host: src 127.0.0.1 → host_veth_ip so the reply routes back here.
        // Match on base_port because DNAT has already rewritten the destination port.
        run("sh", &["-c", &format!(
            "iptables -t nat -C POSTROUTING -o {veth} -s 127.0.0.1 -p tcp --dport {bp} -j SNAT --to-source {hvip} 2>/dev/null || \
             iptables -t nat -A POSTROUTING -o {veth} -s 127.0.0.1 -p tcp --dport {bp} -j SNAT --to-source {hvip}",
            veth = self.veth_host, bp = base_port, hvip = self.host_veth_ip
        )]).await.context("fork SNAT")?;

        Ok(())
    }

    /// Remove host-side iptables/routing rules added by `add_port_forward`.
    pub async fn remove_port_forward(&self, fork_port: u16, base_vm_ip: Ipv4Addr, base_port: u16) {
        let table = fork_port as u32;
        let dest = format!("{}:{}", base_vm_ip, base_port);

        run("ip", &["neigh", "del", &base_vm_ip.to_string(), "dev", &self.veth_host]).await.ok();

        run("sh", &["-c", &format!(
            "iptables -t nat -D OUTPUT -d 127.0.0.1 -p tcp --dport {fp} -j DNAT --to-destination {d} 2>/dev/null || true",
            fp = fork_port, d = dest
        )]).await.ok();

        run("sh", &["-c", &format!(
            "iptables -t nat -D POSTROUTING -o {veth} -s 127.0.0.1 -p tcp --dport {bp} -j SNAT --to-source {hvip} 2>/dev/null || true",
            veth = self.veth_host, bp = base_port, hvip = self.host_veth_ip
        )]).await.ok();

        run("sh", &["-c", &format!(
            "iptables -t mangle -D OUTPUT -d 127.0.0.1 -p tcp --dport {p} -j MARK --set-mark {p} 2>/dev/null || true",
            p = fork_port
        )]).await.ok();

        run("sh", &["-c", &format!(
            "ip route del {vm}/32 table {t} 2>/dev/null || true",
            vm = base_vm_ip, t = table
        )]).await.ok();

        run("sh", &["-c", &format!(
            "ip rule del fwmark {p} table {t} 2>/dev/null || true",
            p = fork_port, t = table
        )]).await.ok();
    }

    /// Tear down the network namespace and all associated interfaces.
    pub async fn destroy(&self) {
        // Deleting the namespace removes veth_ns and the fork TAP inside it.
        run("ip", &["netns", "del", &self.ns_name]).await.ok();
        // Remove the host-side veth (may already be gone if namespace was deleted).
        run("ip", &["link", "del", &self.veth_host]).await.ok();
    }
}

/// Run a command inside a network namespace via `ip netns exec`.
async fn ns_run(ns: &str, prog: &str, args: &[&str]) -> Result<()> {
    let mut full = vec!["netns", "exec", ns, prog];
    full.extend_from_slice(args);
    let status = Command::new("ip")
        .args(&full)
        .status()
        .await
        .with_context(|| format!("ip netns exec {} {} {:?}", ns, prog, args))?;
    if !status.success() {
        anyhow::bail!("ip netns exec {} {} {:?} exited with {}", ns, prog, args, status);
    }
    Ok(())
}

async fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .await
        .with_context(|| format!("failed to spawn {} {:?}", program, args))?;
    if !status.success() {
        anyhow::bail!("{} {:?} exited with {}", program, args, status);
    }
    Ok(())
}
