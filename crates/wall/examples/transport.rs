//! Linux-only two-process QUIC/proxy measurement. Core remains portable.
#[cfg(target_os = "linux")]
mod linux {
    use one_mk_net::{Identity, Node, Pairing};
    use one_mk_wall_sync::{Estimator, Probe, wire::Message};
    use std::{io::{BufRead, BufReader}, process::{Child, Command, Stdio}, sync::Arc, time::Duration};
    use tokio::{net::UdpSocket, task::JoinSet};

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

    fn now() -> Result<f64> {
        let mut timestamp = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut timestamp) };
        if result != 0 { return Err(std::io::Error::last_os_error().into()); }
        Ok(timestamp.tv_sec as f64 + timestamp.tv_nsec as f64 * 1e-9)
    }

    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
    }

    fn node() -> Result<Node> { Ok(Node::bind(Arc::new(Identity::ephemeral()?), "127.0.0.1:0".parse()?)?) }

    async fn pair(peer: &one_mk_net::Peer, node: &Node) -> Result<()> {
        let pairing = Pairing::start(peer, node.identity().fingerprint, "wall-spike").await?;
        if !pairing.confirm(true).await? { return Err("test pairing rejected".into()); }
        Ok(())
    }

    async fn server() -> Result<()> {
        let server = node()?;
        println!("{}", server.local_addr()?);
        let peer = server.accept().await.ok_or("closed")??;
        pair(&peer, &server).await?;
        loop {
            let (mut send, mut recv) = peer.conn.accept_bi().await?;
            let mut bytes = [0u8; 64];
            recv.read_exact(&mut bytes).await?;
            let mut message = Message::decode(&bytes).map_err(|error| format!("decode {error:?}"))?;
            if message.reply { return Err("unexpected response".into()); }
            message.probe.t2 = now()? + 3.0;
            message.reply = true;
            message.probe.t3 = now()? + 3.0;
            send.write_all(&message.encode().map_err(|error| format!("encode {error:?}"))?).await?;
            send.finish()?;
        }
    }

    async fn measure(model: &str) -> Result<()> {
        let mut child = Process(Command::new(std::env::current_exe()?).arg("server").stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?);
        let stdout = child.0.stdout.take().ok_or("child stdout")?;
        let mut address = String::new();
        BufReader::new(stdout).read_line(&mut address)?;
        let server_addr: std::net::SocketAddr = address.trim().parse()?;
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
        let relay_addr = socket.local_addr()?;
        let model_owned = model.to_owned();
        let relay = tokio::spawn(async move {
            let mut client = None;
            let mut bytes = [0u8; 65536];
            let mut pending = JoinSet::new();
            let mut sequence = 0u64;
            loop {
                tokio::select! {
                    result = socket.recv_from(&mut bytes) => {
                        let (length, source) = result?;
                        let reverse = source == server_addr;
                        let destination = if reverse { client.ok_or(std::io::Error::other("no client"))? } else { client = Some(source); server_addr };
                        sequence = sequence.wrapping_add(1);
                        let jitter = ((sequence.wrapping_mul(6364136223846793005) >> 32) % 4000) as f64 / 1e6;
                        let delay = match model_owned.as_str() {
                            "wired" => 0.0002,
                            "wifi" => 0.001 + jitter,
                            "bad" => 0.001 + jitter + if reverse { 0.006 } else { 0.0 } + if sequence.is_multiple_of(13) { 0.080 } else { 0.0 },
                            _ => if reverse { 0.0 } else { 0.120 + jitter },
                        };
                        if model_owned == "bad" && sequence.is_multiple_of(29) { continue; }
                        if pending.len() >= 256 { continue; }
                        let packet = bytes[..length].to_vec();
                        let socket = socket.clone();
                        pending.spawn(async move {
                            tokio::time::sleep(Duration::from_secs_f64(delay)).await;
                            socket.send_to(&packet, destination).await
                        });
                    }
                    _ = pending.join_next(), if !pending.is_empty() => (),
                }
            }
            #[allow(unreachable_code)]
            Ok::<(), std::io::Error>(())
        });
        let client = node()?;
        let result = async {
            let peer = client.connect_timeout(relay_addr, Duration::from_secs(10)).await?;
            pair(&peer, &client).await?;
            let mut estimator = Estimator::new(1, 1);
            let mut errors = Vec::new();
            let mut rtts = Vec::new();
            for burst in 0..20u64 {
                let mut probes = Vec::new();
                for sample in 0..8u64 {
                    let sequence = burst * 8 + sample;
                    let (mut send, mut recv) = peer.conn.open_bi().await?;
                    let request = Message { reply: false, probe: Probe { session: 1, generation: 1, sequence, t1: now()?, t2: 0.0, t3: 0.0, t4: 0.0 } };
                    send.write_all(&request.encode().map_err(|error| format!("encode {error:?}"))?).await?;
                    send.finish()?;
                    let mut bytes = [0u8; 64];
                    recv.read_exact(&mut bytes).await?;
                    let arrival = now()?;
                    let reply = Message::decode(&bytes).map_err(|error| format!("decode {error:?}"))?;
                    if !reply.reply || reply.probe.sequence != sequence || reply.probe.t1 != request.probe.t1 || reply.probe.generation != 1 || reply.probe.session != 1 { return Err("unmatched response".into()); }
                    let probe = Probe { t4: arrival, ..reply.probe };
                    rtts.push(probe.measurement().map_err(|error| format!("measurement {error:?}"))?.2 * 1000.0);
                    probes.push(probe);
                }
                estimator.observe(&probes).map_err(|error| format!("observe {error:?}"))?;
                let local = now()?;
                errors.push((estimator.coordinator_time(local).map_err(|error| format!("map {error:?}"))? - local - 3.0).abs() * 1000.0);
            }
            errors.sort_by(f64::total_cmp);
            rtts.sort_by(f64::total_cmp);
            println!("{model},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}", errors.len(), errors.iter().sum::<f64>() / errors.len() as f64, errors[10], errors[18], errors[19], errors[19], rtts[80], rtts[152]);
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }.await;
        client.shutdown(Duration::from_millis(100)).await;
        relay.abort();
        let _ = relay.await;
        result
    }

    pub async fn run() -> Result<()> {
        if std::env::args().nth(1).as_deref() == Some("server") { return server().await; }
        println!("model,bursts,mean_abs_ms,p50_ms,p95_ms,p99_ms,max_ms,rtt_p50_ms,rtt_p95_ms");
        for model in ["wired", "wifi", "bad", "adversarial"] {
            tokio::time::timeout(Duration::from_secs(120), measure(model)).await??;
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> { linux::run().await }

#[cfg(not(target_os = "linux"))]
fn main() { eprintln!("The real transport experiment requires Linux CLOCK_MONOTONIC."); }
