use std::collections::HashMap;
use std::fmt;
use std::iter::FromIterator;
use std::slice;

use bitcoin::hashes::sha256d::{Hash as Sha256dHash, Hash};
use bitcoin::util::hash::BitcoinHash;
use time;

#[cfg(not(feature = "liquid"))]
use bitcoin::consensus::encode::serialize;
#[cfg(feature = "liquid")]
use elements::encode::serialize;

use crate::chain::{Block, BlockHeader};
use crate::errors::*;
use crate::new_index::{BlockEntry, ScriptStats};
use crate::rest::{BlockValue, TransactionValue, UtxoValue};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BlockId {
    pub height: usize,
    pub hash: Sha256dHash,
    pub time: u32,
}

impl From<&HeaderEntry> for BlockId {
    fn from(header: &HeaderEntry) -> Self {
        BlockId {
            height: header.height(),
            hash: *header.hash(),
            time: header.header().time,
        }
    }
}

#[derive(Eq, PartialEq, Clone)]
pub struct HeaderEntry {
    height: usize,
    hash: Sha256dHash,
    header: BlockHeader,
}

impl HeaderEntry {
    pub fn hash(&self) -> &Sha256dHash {
        &self.hash
    }

    pub fn header(&self) -> &BlockHeader {
        &self.header
    }

    pub fn height(&self) -> usize {
        self.height
    }
}

impl fmt::Debug for HeaderEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let last_block_time = time::at_utc(time::Timespec::new(self.header().time as i64, 0))
            .rfc3339()
            .to_string();
        write!(
            f,
            "best={} height={} @ {}",
            self.hash(),
            self.height(),
            last_block_time,
        )
    }
}

pub struct HeaderList {
    headers: Vec<HeaderEntry>,
    heights: HashMap<Sha256dHash, usize>,
    tip: Sha256dHash,
}

impl HeaderList {
    pub fn empty() -> HeaderList {
        HeaderList {
            headers: vec![],
            heights: HashMap::new(),
            tip: Sha256dHash::default(),
        }
    }

    pub fn new(
        mut headers_map: HashMap<Sha256dHash, BlockHeader>,
        tip_hash: Sha256dHash,
    ) -> HeaderList {
        trace!(
            "processing {} headers, tip at {:?}",
            headers_map.len(),
            tip_hash
        );

        let mut blockhash = tip_hash;
        let mut headers_chain = vec![];
        let null_hash = Sha256dHash::default();

        while blockhash != null_hash {
            let header = headers_map.remove(&blockhash).expect(&format!(
                "missing expected blockhash in headers map: {:?}",
                blockhash
            ));
            blockhash = header.prev_blockhash.clone();
            headers_chain.push(header);
        }
        headers_chain.reverse();

        trace!(
            "{} chained headers ({} orphan blocks left)",
            headers_chain.len(),
            headers_map.len()
        );

        let mut headers = HeaderList::empty();
        headers.apply(headers.order(headers_chain));
        headers
    }

    pub fn order(&self, new_headers: Vec<BlockHeader>) -> Vec<HeaderEntry> {
        // header[i] -> header[i-1] (i.e. header.last() is the tip)
        struct HashedHeader {
            blockhash: Sha256dHash,
            header: BlockHeader,
        }
        let hashed_headers =
            Vec::<HashedHeader>::from_iter(new_headers.into_iter().map(|header| HashedHeader {
                blockhash: header.bitcoin_hash(),
                header,
            }));
        for i in 1..hashed_headers.len() {
            assert_eq!(
                hashed_headers[i].header.prev_blockhash,
                hashed_headers[i - 1].blockhash
            );
        }
        let prev_blockhash = match hashed_headers.first() {
            Some(h) => h.header.prev_blockhash,
            None => return vec![], // hashed_headers is empty
        };
        let null_hash = Sha256dHash::default();
        let new_height: usize = if prev_blockhash == null_hash {
            0
        } else {
            self.header_by_blockhash(&prev_blockhash)
                .expect(&format!("{} is not part of the blockchain", prev_blockhash))
                .height()
                + 1
        };
        (new_height..)
            .zip(hashed_headers.into_iter())
            .map(|(height, hashed_header)| HeaderEntry {
                height: height,
                hash: hashed_header.blockhash,
                header: hashed_header.header,
            })
            .collect()
    }

    pub fn apply(&mut self, new_headers: Vec<HeaderEntry>) {
        // new_headers[i] -> new_headers[i - 1] (i.e. new_headers.last() is the tip)
        for i in 1..new_headers.len() {
            assert_eq!(new_headers[i - 1].height() + 1, new_headers[i].height());
            assert_eq!(
                *new_headers[i - 1].hash(),
                new_headers[i].header().prev_blockhash
            );
        }
        let new_height = match new_headers.first() {
            Some(entry) => {
                let height = entry.height();
                let expected_prev_blockhash = if height > 0 {
                    *self.headers[height - 1].hash()
                } else {
                    Sha256dHash::default()
                };
                assert_eq!(entry.header().prev_blockhash, expected_prev_blockhash);
                height
            }
            None => return,
        };
        debug!(
            "applying {} new headers from height {}",
            new_headers.len(),
            new_height
        );
        self.headers.split_off(new_height); // keep [0..new_height) entries
        for new_header in new_headers {
            let height = new_header.height();
            assert_eq!(height, self.headers.len());
            self.tip = *new_header.hash();
            self.headers.push(new_header);
            self.heights.insert(self.tip, height);
        }
    }

    pub fn header_by_blockhash(&self, blockhash: &Sha256dHash) -> Option<&HeaderEntry> {
        let height = self.heights.get(blockhash)?;
        let header = self.headers.get(*height)?;
        if *blockhash == *header.hash() {
            Some(header)
        } else {
            None
        }
    }

    pub fn header_by_height(&self, height: usize) -> Option<&HeaderEntry> {
        self.headers.get(height).map(|entry| {
            assert_eq!(entry.height(), height);
            entry
        })
    }

    pub fn equals(&self, other: &HeaderList) -> bool {
        self.headers.last() == other.headers.last()
    }

    pub fn tip(&self) -> &Sha256dHash {
        assert_eq!(
            self.tip,
            self.headers
                .last()
                .map(|h| *h.hash())
                .unwrap_or(Sha256dHash::default())
        );
        &self.tip
    }

    pub fn len(&self) -> usize {
        self.headers.len()
    }

    pub fn iter(&self) -> slice::Iter<HeaderEntry> {
        self.headers.iter()
    }
}

#[derive(Serialize, Deserialize)]
pub struct BlockStatus {
    pub in_best_chain: bool,
    pub height: Option<usize>,
    pub next_best: Option<Sha256dHash>,
}

impl BlockStatus {
    pub fn confirmed(height: usize, next_best: Option<Sha256dHash>) -> BlockStatus {
        BlockStatus {
            in_best_chain: true,
            height: Some(height),
            next_best,
        }
    }

    pub fn orphaned() -> BlockStatus {
        BlockStatus {
            in_best_chain: false,
            height: None,
            next_best: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BlockMeta {
    pub tx_count: u32,
    pub size: u32,
    pub weight: u32,
}

pub struct BlockHeaderMeta {
    pub header_entry: HeaderEntry,
    pub meta: BlockMeta,
}

impl From<&Block> for BlockMeta {
    fn from(block: &Block) -> BlockMeta {
        BlockMeta {
            tx_count: block.txdata.len() as u32,
            weight: block.txdata.iter().map(|tx| tx.get_weight() as u32).sum(),
            size: serialize(block).len() as u32,
        }
    }
}

impl From<&BlockEntry> for BlockMeta {
    fn from(b: &BlockEntry) -> BlockMeta {
        BlockMeta {
            tx_count: b.block.txdata.len() as u32,
            weight: b.block.txdata.iter().map(|tx| tx.get_weight() as u32).sum(),
            size: b.size,
        }
    }
}

impl BlockMeta {
    pub fn parse_getblock(val: ::serde_json::Value) -> Result<BlockMeta> {
        Ok(BlockMeta {
            tx_count: val
                .get("nTx")
                .chain_err(|| "missing nTx")?
                .as_f64()
                .chain_err(|| "nTx not a number")? as u32,
            size: val
                .get("size")
                .chain_err(|| "missing size")?
                .as_f64()
                .chain_err(|| "size not a number")? as u32,
            weight: val
                .get("weight")
                .chain_err(|| "missing weight")?
                .as_f64()
                .chain_err(|| "weight not a number")? as u32,
        })
    }
}

#[derive(Serialize)]
pub struct AddressInfo {
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chain_stats: Option<ScriptStats>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub mempool_stats: Option<ScriptStats>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub utxo: Vec<UtxoValue>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub chain_txs: Vec<TransactionValue>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub mempool_txs: Vec<TransactionValue>,
}

impl AddressInfo {
    pub fn new(
        address: String,
        stats: (ScriptStats, ScriptStats),
        chain_txs: Vec<TransactionValue>,
        mempool_txs: Vec<TransactionValue>,
    ) -> AddressInfo {
        AddressInfo {
            address,
            chain_stats: Some(stats.0),
            mempool_stats: Some(stats.1),
            utxo: vec![],
            chain_txs,
            mempool_txs,
        }
    }

    pub fn new_stats(address: String, stats: (ScriptStats, ScriptStats)) -> AddressInfo {
        AddressInfo {
            address,
            utxo: vec![],
            chain_stats: Some(stats.0),
            mempool_stats: Some(stats.1),
            chain_txs: vec![],
            mempool_txs: vec![],
        }
    }

    pub fn new_utxo(address: String, utxos: Vec<UtxoValue>) -> AddressInfo {
        AddressInfo {
            address,
            utxo: utxos,
            chain_stats: None,
            mempool_stats: None,
            chain_txs: vec![],
            mempool_txs: vec![],
        }
    }
}

#[derive(Serialize)]
pub struct BlockInfo {
    pub block: BlockValue,
    pub transactions: Vec<TransactionValue>,
}

impl BlockInfo {
    pub fn new(block: BlockValue, transactions: Vec<TransactionValue>) -> BlockInfo {
        BlockInfo {
            block,
            transactions,
        }
    }
}

#[derive(Serialize)]
pub struct BlockHashInfo {
    pub block: String,
    pub transactions: Vec<Hash>,
}

impl BlockHashInfo {
    pub fn new(block: String, transactions: Vec<Hash>) -> BlockHashInfo {
        BlockHashInfo {
            block,
            transactions,
        }
    }
}
