use bitcoincash_addr::{Address, Scheme, AddressError};
use std::env;
use bitcoincash_addr::Base58Error::InvalidLength;

fn main() {
    let args: Vec<String> = env::args().collect();

    let result = match args.get(1) {
        Some(addr) => to_legacy(addr),
        None => Err(AddressError::from(InvalidLength(0))),
    };

    match result {
        Ok(addr) => println!("Legacy address: {}", addr),
        Err(e) => println!("Error: {}", e),
    };
}

fn to_legacy(addr: &String) -> Result<String, AddressError> {
    let mut addr = Address::decode(addr)?;
    addr.scheme = Scheme::Base58;
    return addr.encode();
}