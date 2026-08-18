//! Offline signing helper for `scripts/publish-release.ps1`.
//!
//! The private seed is deliberately read from an ignored local file. This
//! binary only emits a base64 signature and refuses a key whose public half is
//! not one of the release keys compiled into the client.

use anyhow::{bail, Context, Result};
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};

fn decode_seed(raw: &str) -> Result<[u8; 32]> {
    let hex = raw.trim();
    if hex.len() != 64 {
        bail!("la clé de publication doit être une graine Ed25519 hexadécimale de 32 octets");
    }
    let mut seed = [0u8; 32];
    for (slot, pair) in seed.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).expect("hex is ASCII");
        *slot = u8::from_str_radix(pair, 16)
            .with_context(|| "la clé de publication contient un caractère non hexadécimal")?;
    }
    Ok(seed)
}

fn sign(key_path: &str, manifest_path: &str) -> Result<String> {
    let seed = decode_seed(
        &std::fs::read_to_string(key_path)
            .with_context(|| format!("lecture de la clé {key_path}"))?,
    )?;
    let key = SigningKey::from_bytes(&seed);
    let public_hex = key
        .verifying_key()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if !luavault_lib::RELEASE_PUBLIC_KEYS
        .iter()
        .any(|allowed| *allowed == public_hex)
    {
        bail!("la clé privée ne correspond à aucune clé publique compilée");
    }

    let manifest = std::fs::read(manifest_path)
        .with_context(|| format!("lecture du manifeste {manifest_path}"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(key.sign(&manifest).to_bytes()))
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [command, key_path, manifest_path] = args.as_slice() else {
        bail!("usage: lvrelease sign <clé-privée> <manifest.json>");
    };
    if command != "sign" {
        bail!("commande inconnue : {command}");
    }
    println!("{}", sign(key_path, manifest_path)?);
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("lvrelease: {error:#}");
        std::process::exit(1);
    }
}
