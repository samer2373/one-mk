//! `one-mk pair`: find another computer running `one-mk pair`, show the
//! same 6-digit code on both, and remember each other once confirmed.

use anyhow::{Context, Result};
use one_mk_net::{Device, Discovery, Identity, Node, Pairing, Peer, TrustStore};
use one_mk_proto::Side;
use std::collections::HashSet;
use std::io::Write;
use std::net::{Ipv6Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub async fn run(dir: &Path, connect: Option<SocketAddr>) -> Result<()> {
    let identity = Arc::new(Identity::load_or_create(dir)?);
    let mut trust = TrustStore::load(dir)?;
    let node = Node::bind(identity.clone(), SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)))
        .or_else(|_| Node::bind(identity.clone(), SocketAddr::from(([0, 0, 0, 0], 0))))?;
    let port = node.local_addr()?.port();
    let (_discovery, mut found) = Discovery::start(&identity, port, true)?;
    let me = identity.fingerprint;

    println!("This computer: {} ({})", identity.name, me.short());
    println!("Looking for another computer... run `one-mk pair` on it too.");

    let (tx, mut peers) = mpsc::channel::<Peer>(4);
    {
        let (node, tx) = (node.clone(), tx.clone());
        tokio::spawn(async move {
            while let Some(res) = node.accept().await {
                if let Ok(p) = res
                    && tx.send(p).await.is_err()
                {
                    break;
                }
            }
        });
    }
    if let Some(addr) = connect {
        dial(node.clone(), vec![addr], tx.clone());
    }

    let mut dialed = HashSet::new();
    let peer = loop {
        tokio::select! {
            Some(f) = found.recv() => {
                // Only the lower id dials, so the two computers never race
                // each other with two separate pairing sessions.
                if f.pairing && f.fingerprint > me && dialed.insert(f.fingerprint) {
                    dial(node.clone(), f.addrs, tx.clone());
                }
            }
            Some(p) = peers.recv() => break p,
        }
    };

    // Remember the address with the service port (not this temporary one),
    // for networks where discovery is blocked.
    let addr = SocketAddr::new(peer.conn.remote_address().ip().to_canonical(), one_mk_proto::DEFAULT_PORT);
    let fingerprint = peer.fingerprint;
    let initiator = peer.initiator;
    let pairing = Pairing::start(&peer, me, &identity.name).await.context("pairing failed")?;
    let name = pairing.peer_name.clone();
    println!();
    println!("  Found {name} ({})", fingerprint.short());
    println!();
    println!("      {}", pairing.sas);
    println!();
    println!("  Check that {name} shows the same code.");
    let ok = ask("  Do the codes match? [y/N] ").await?;
    if !ok {
        let _ = pairing.confirm(false).await;
        println!("Pairing cancelled.");
        return Ok(());
    }
    println!("  Waiting for {name} to confirm...");
    if !pairing.confirm(true).await? {
        println!("{name} did not confirm. Nothing was saved.");
        return Ok(());
    }

    // Opposite defaults on each side, so the layout agrees out of the box.
    let side = if initiator { Side::Right } else { Side::Left };
    trust.upsert(Device {
        name: name.clone(),
        fingerprint: fingerprint.to_string(),
        side: side.to_string(),
        addrs: vec![addr.to_string()],
    });
    trust.save()?;
    let side = trust.get(&fingerprint).map_or(side, |d| d.side());
    println!();
    println!("Paired with {name}. It is set to the {side} of this computer.");
    println!("If that's wrong:  one-mk layout \"{name}\" {}", side.opposite());
    println!("Start sharing:    one-mk run   (on both computers)");
    node.close();
    tokio::time::sleep(Duration::from_millis(50)).await;
    Ok(())
}

fn dial(node: Node, addrs: Vec<SocketAddr>, tx: mpsc::Sender<Peer>) {
    tokio::spawn(async move {
        for addr in addrs {
            if let Ok(Ok(p)) = tokio::time::timeout(Duration::from_secs(3), node.connect(addr)).await {
                let _ = tx.send(p).await;
                return;
            }
        }
    });
}

async fn ask(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let line = tokio::task::spawn_blocking(|| {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s).map(|_| s)
    })
    .await??;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}
