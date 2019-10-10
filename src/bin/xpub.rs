use bitcoin::util::bip32::{DerivationPath, ExtendedPubKey};
use secp256k1::{self, Secp256k1};
use std::env;
use std::str::FromStr;

fn main() {
    let args: Vec<String> = env::args().collect();
    let xpub = args.get(1).unwrap();

    let secp = Secp256k1::new();
    let path = DerivationPath::from_str("m/0/1").unwrap();

    let key = ExtendedPubKey::from_str(xpub).unwrap();
    let addr = key.derive_pub(&secp, &path).unwrap();

    println!("Address: {}", addr.public_key);
}