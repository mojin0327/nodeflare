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
