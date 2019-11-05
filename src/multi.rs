use crate::chain::address;
use crate::config::Config;
use crate::new_index::{Query, ScriptStats};
use crate::rest::{prepare_txs, to_scripthash, UtxoValue, CHAIN_TXS_PER_PAGE, MAX_MEMPOOL_TXS};
use crate::util::{AddressInfo, FullHash};

use bitcoin::network::constants::Network::Bitcoin;
use bitcoin::util::bip32::{DerivationPath, ExtendedPubKey};
use secp256k1::{self, Secp256k1};
use std::borrow::Borrow;
use std::str::FromStr;

const MULTIADDR_SEPARATOR: &str = "%7C";
const DERIVE_SIZE: u32 = 100;
const XPUB_PREFIX: &str = "xpub";

pub fn xpub_multi_or_single(input: &str) -> (Vec<String>, bool) {
    if input.starts_with(XPUB_PREFIX) {
        // Return empty vector, addresses will be derived in `handle_xpub()`
        return (vec![], true);
    } else if input.contains(MULTIADDR_SEPARATOR) {
        // Return mutliple addresses
        let addresses = input
            .split(MULTIADDR_SEPARATOR)
            .into_iter()
            .map(|i| i.to_owned())
            .collect();
        return (addresses, false);
    }

    // Return single address
    return (vec![input.to_owned()], false);
}

fn derive_batch(
    input: ExtendedPubKey,
    page: u32,
    secp: &secp256k1::Secp256k1<secp256k1::All>,
    config: &Config,
) -> Vec<(String, FullHash)> {
    // Page 1: 0-99
    // Page 2: 100-199
    // Page 3: 200-299
    // ..
    let from: u32 = (page - 1) * DERIVE_SIZE;
    let to: u32 = (page * DERIVE_SIZE) - 1;

    let addresses: Vec<(String, FullHash)> = (from..to)
        .map(|i| derive_by_index(input, i, &secp, config))
        .collect();

    return addresses;
}

fn derive_by_index(
    xpub: ExtendedPubKey,
    i: u32,
    secp: &secp256k1::Secp256k1<secp256k1::All>,
    config: &Config,
) -> (String, FullHash) {
    debug!("Deriving address number {}", i);
    let path = format!("m/0/{}", i);
    let path_ref = path.as_ref();
    let derivation = DerivationPath::from_str(path_ref).unwrap();

    let child = xpub.derive_pub(secp, &derivation).unwrap();
    let p2pkh = address::Address::p2pkh(child.public_key.borrow(), Bitcoin);
    let address = p2pkh.to_string();

    let hash = to_scripthash("address", address.as_str(), &config.network_type);
    return (address, hash.unwrap());
}

pub fn handle_xpub_info(input: ExtendedPubKey, query: &Query, config: &Config) -> Vec<AddressInfo> {
    return handle_xpub_inner(input, query, config, get_address_info);
}

pub fn handle_xpub_stats(
    input: ExtendedPubKey,
    query: &Query,
    config: &Config,
) -> Vec<AddressInfo> {
    return handle_xpub_inner(input, query, config, get_address_stats);
}

pub fn handle_xpub_utxo(input: ExtendedPubKey, query: &Query, config: &Config) -> Vec<AddressInfo> {
    return handle_xpub_inner(input, query, config, get_address_utxo);
}

pub fn handle_multiaddr_info(
    addresses: Vec<String>,
    query: &Query,
    config: &Config,
) -> Vec<AddressInfo> {
    return handle_multiaddr_inner(addresses, query, config, get_address_info);
}

pub fn handle_multiaddr_stats(
    addresses: Vec<String>,
    query: &Query,
    config: &Config,
) -> Vec<AddressInfo> {
    return handle_multiaddr_inner(addresses, query, config, get_address_stats);
}

pub fn handle_multiaddr_utxo(
    addresses: Vec<String>,
    query: &Query,
    config: &Config,
) -> Vec<AddressInfo> {
    return handle_multiaddr_inner(addresses, query, config, get_address_utxo);
}

fn handle_multiaddr_inner(
    addresses: Vec<String>,
    query: &Query,
    config: &Config,
    callback: fn(String, FullHash, (ScriptStats, ScriptStats), &Query, &Config) -> AddressInfo,
) -> Vec<AddressInfo> {
    return addresses
        .into_iter()
        .map(|addr| {
            let addr_ref = addr.as_ref();
            let result = to_scripthash("address", addr_ref, &config.network_type);
            return (addr, result);
        })
        .filter_map(|(addr, result)| match result {
            Ok(hash) => Some((addr, hash)),
            Err(_) => None,
        })
        .map(|(addr, hash)| {
            let stats = query.stats(&hash[..]);
            let data = callback(addr, hash, stats, query, config);
            return data;
        })
        .collect();
}

fn handle_xpub_inner(
    input: ExtendedPubKey,
    query: &Query,
    config: &Config,
    callback: fn(String, FullHash, (ScriptStats, ScriptStats), &Query, &Config) -> AddressInfo,
) -> Vec<AddressInfo> {
    // Return first derived 100 xpub
    let mut result: Vec<AddressInfo> = vec![];
    let secp = Secp256k1::new();

    let mut page: u32 = 1;
    let mut is_empty: bool;
    let mut empty_count: u32 = 0;
    let mut done: bool = false;

    loop {
        debug!("Deriving batch number {}", page);
        let addresses = derive_batch(input, page, &secp, &config);

        for (addr, hash) in addresses {
            // Grab stats to check if unused address
            let stats = query.stats(&hash[..]);
            if stats.0.is_empty() && stats.1.is_empty() {
                debug!("Address {} is unused", addr);
                is_empty = true;
                empty_count += 1;
            } else {
                debug!("Address {} is used", addr);
                is_empty = false;
                empty_count = 0;
            }

            // Grab transactions for used address
            if !is_empty {
                let data = callback(addr, hash, stats, query, config);
                result.push(data);
            }

            // Stop if chain of 20 unused addresses found
            if is_empty && empty_count >= 20 {
                debug!("Chain of 20 unused addresses found, stopping scan...");
                done = true;
                break;
            }
        }

        page += 1;

        if done {
            break;
        }
    }

    return result;
}

fn get_address_info(
    addr: String,
    hash: FullHash,
    stats: (ScriptStats, ScriptStats),
    query: &Query,
    config: &Config,
) -> AddressInfo {
    let chain_txs_raw = query
        .chain()
        .history(&hash, None, CHAIN_TXS_PER_PAGE)
        .into_iter()
        .map(|(tx, blockid)| (tx, Some(blockid)))
        .collect();

    let mempool_txs_raw = query
        .mempool()
        .history(&hash, MAX_MEMPOOL_TXS)
        .into_iter()
        .map(|tx| (tx, None))
        .collect();

    let chain_txs = prepare_txs(chain_txs_raw, query, config);
    let mempool_txs = prepare_txs(mempool_txs_raw, query, config);
    return AddressInfo::new(addr, stats, chain_txs, mempool_txs);
}

fn get_address_stats(
    addr: String,
    _hash: FullHash,
    stats: (ScriptStats, ScriptStats),
    _query: &Query,
    _config: &Config,
) -> AddressInfo {
    return AddressInfo::new_stats(addr, stats);
}

fn get_address_utxo(
    addr: String,
    hash: FullHash,
    _stats: (ScriptStats, ScriptStats),
    query: &Query,
    _config: &Config,
) -> AddressInfo {
    let utxos: Vec<UtxoValue> = query
        .utxo(&hash[..])
        .into_iter()
        .map(UtxoValue::from)
        .collect();

    return AddressInfo::new_utxo(addr, utxos);
}
