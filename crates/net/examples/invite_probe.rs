use anyhow::{Context, Result};
use one_mk_net::{Fingerprint, Identity, Node};
use one_mk_proto::StreamKind;
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::sync::Arc;
use std::io::Write;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    anyhow::ensure!((4..=5).contains(&arguments.len()), "usage: invite_probe <numeric address:port> <public host fingerprint> <public expiry> [slow]");
    let address: SocketAddr = arguments[1].parse()?;
    let fingerprint: Fingerprint = arguments[2].parse()?;
    let expiry: u64 = arguments[3].parse()?;
    if arguments.get(4).is_some_and(|mode| mode == "slow") {
        let mut nodes = Vec::new();
        let mut peers = Vec::new();
        for _ in 0..5 {
            let node = Node::bind_any(Arc::new(Identity::ephemeral()?), 0)?;
            for _ in 0..4 { peers.push(node.connect_pinned(address, fingerprint).await?); }
            nodes.push(node);
        }
        println!("Slow-loris: holding 20 silent TLS connections across 5 certificates; no magic, ID or secret supplied.");
        std::io::stdout().flush()?;
        tokio::time::sleep(Duration::from_secs(12)).await;
        for peer in peers { peer.conn.close(0u32.into(), b"probe complete"); }
        for node in nodes { node.shutdown(Duration::from_millis(50)).await; }
        return Ok(());
    }
    let mut public = vec![1];
    public.extend_from_slice(&fingerprint.0);
    public.extend_from_slice(&expiry.to_be_bytes());
    public.extend_from_slice(&24900u16.to_be_bytes());
    let hostname = format!("one-mk-{}.local.", fingerprint.short());
    field(&mut public, hostname.as_bytes());
    public.push(1);
    field(&mut public, address.to_string().as_bytes());
    let public_id: [u8; 32] = Sha256::digest(public).into();
    let node = Node::bind_any(Arc::new(Identity::ephemeral()?), 0)?;
    let mut refused = 0;
    let mut busy = 0;
    let result: Result<()> = async {
        for include_proof in [false, true] {
            for _ in 0..1000 {
                let attempt: Result<()> = async {
                    let peer = node.connect_pinned(address, fingerprint).await?;
                    let result: Result<()> = async {
                        let (mut send, mut recv) = peer.open_stream(StreamKind::Pairing).await?;
                        let mut bytes = b"\xff\xffOMKI\x01".to_vec();
                        bytes.extend_from_slice(&public_id);
                        field(&mut bytes, b"public attacker");
                        bytes.extend_from_slice(&24901u16.to_be_bytes());
                        bytes.extend_from_slice(&[0; 32]);
                        if include_proof { bytes.extend_from_slice(&[0; 32]); }
                        send.write_all(&bytes).await?;
                        let mut response = [255];
                        recv.read_exact(&mut response).await?;
                        anyhow::ensure!(response == [0], "public-data invitation ID was accepted");
                        refused += 1;
                        Ok(())
                    }.await;
                    peer.conn.close(0u32.into(), b"probe complete");
                    result
                }.await;
                if let Err(error) = attempt {
                    if format!("{error:#}").contains("busy") { busy += 1; }
                    else { return Err(error).context("public-only probe failed unexpectedly"); }
                }
            }
        }
        Ok(())
    }.await;
    node.shutdown(Duration::from_millis(200)).await;
    result?;
    println!("Public-only attacker: 1000 wrong-ID + 1000 wrong-proof/wrong-ID attempts; {refused} explicitly refused, {busy} source-rate-limited; no secret supplied.");
    Ok(())
}

fn field(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value);
}
