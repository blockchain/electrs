use bitcoin::util::bip32::{DerivationPath, ExtendedPubKey};
use secp256k1::{self, Secp256k1};
use std::env;
use std::str::FromStr;
use bitcoin::util::address;
use bitcoin::network::constants::Network::Bitcoin;
use std::borrow::Borrow;

fn main() {
    let args: Vec<String> = env::args().collect();
    let xpub = args.get(1).unwrap();

    let secp = Secp256k1::new();
    let path = DerivationPath::from_str("m/0/0").unwrap();

    let key = ExtendedPubKey::from_str(xpub).unwrap();
    let child = key.derive_pub(&secp, &path).unwrap();
    let address = address::Address::p2pkh(child.public_key.borrow(), Bitcoin).to_string();

    println!("Address: {}", address);
}