use tokio::time::delay_for;
use std::time::Duration;
use structopt::StructOpt;

use anyhow::{ensure, Result};
use reqwest::Url;

use diem_types::{
    chain_id::ChainId,
    ledger_info::LedgerInfoWithSignatures,
    epoch_change::EpochChangeProof,
    proof::{
        AccumulatorConsistencyProof,
    },
    trusted_state::{TrustedState, TrustedStateChange},
};
use diem_json_rpc_client::{
    get_response_from_batch,
    views::{
        AccountStateWithProofView, AccountView, BytesView, CurrencyInfoView,
        EventView, StateProofView, TransactionView, TransactionDataView
    },
    JsonRpcBatch, JsonRpcClient, JsonRpcResponse, ResponseAsView,
};
use std::{convert::TryFrom};
use diem_json_rpc_types::views::AmountView;
use diem_types::account_state_blob::{AccountStateWithProof, AccountStateBlob};

#[derive(Debug, StructOpt)]
#[structopt(name = "pDiem")]
struct Args {
    #[structopt(
    default_value = "http://127.0.0.1:8080", long,
    help = "Diem rpc endpoint")]
    diem_rpc_endpoint: String,
}

pub struct LibraDemo {
    chain_id: ChainId,
    rpc_client: JsonRpcClient,
    trusted_state: Option<TrustedState>,
    latest_epoch_change_li: Option<LedgerInfoWithSignatures>,
    latest_li: Option<LedgerInfoWithSignatures>,
    sent_events_key: Option<BytesView>,
    received_events_key:Option<BytesView>,
    sent_events: Option<Vec<EventView>>,
    received_events: Option<Vec<EventView>>,
    transactions: Option<Vec<TransactionView>>,
    //account: Option<AccountData>,
    balances: Option<Vec<AmountView>>,
}
impl LibraDemo {
    pub fn new(url: &str) -> Result<Self> {
        let rpc_client = JsonRpcClient::new(Url::parse(url).unwrap()).unwrap();
        Ok(LibraDemo {
            chain_id: ChainId::new(2),
            rpc_client,
            sent_events_key: None,
            received_events_key: None,
            trusted_state: None,
            latest_epoch_change_li: None,
            latest_li: None,
            sent_events: None,
            received_events: None,
            transactions:None,
            //account: None,
            balances: None,
        })
    }

    pub fn init_state(
        &mut self,
        from_version: u64
    ) -> Result<()> {
        let mut batch = JsonRpcBatch::new();
        batch.add_get_state_proof_request(from_version);

        let responses = self.rpc_client.execute(batch).unwrap();

        let resp = get_response_from_batch(0, &responses).unwrap().as_ref().unwrap();

        let mut state_proof = StateProofView::from_response(resp.clone()).unwrap();
        println!("state_proof:\n{:?}", state_proof);

        let epoch_change_proof: EpochChangeProof =
            bcs::from_bytes(&state_proof.epoch_change_proof.into_bytes().unwrap()).unwrap();
        let ledger_info_with_signatures: LedgerInfoWithSignatures =
            bcs::from_bytes(&state_proof.ledger_info_with_signatures.into_bytes().unwrap()).unwrap();

        let ledger_consistency_proof: AccumulatorConsistencyProof =
            bcs::from_bytes(&state_proof.ledger_consistency_proof.into_bytes().unwrap()).unwrap();
        // Init zero version state
        let zero_ledger_info_with_sigs = epoch_change_proof.ledger_info_with_sigs[0].clone();

        self.latest_epoch_change_li = Option::from(zero_ledger_info_with_sigs.clone());
        self.trusted_state = Option::from(TrustedState::try_from(zero_ledger_info_with_sigs.ledger_info()).unwrap());
        self.latest_li = Option::from(ledger_info_with_signatures.clone());

        // Update Latest version state
        let _ = self.verify_state_proof(ledger_info_with_signatures, epoch_change_proof);
        println!("{:#?}", self.trusted_state);
        println!("{:#?}", self.latest_li);
        println!("{:#?}", self.latest_epoch_change_li);
        Ok(())
    }

    pub fn verify_state_proof(
        &mut self,
        li: LedgerInfoWithSignatures,
        epoch_change_proof: EpochChangeProof
    ) -> Result<()> {
        let client_version = self.trusted_state.as_mut().unwrap().latest_version();
        // check ledger info version
        ensure!(
            li.ledger_info().version() >= client_version,
            "Got stale ledger_info with version {}, known version: {}",
            li.ledger_info().version(),
            client_version,
        );

        // trusted_state_change
        match self
            .trusted_state
            .as_mut()
            .unwrap()
            .verify_and_ratchet(&li, &epoch_change_proof)?
        {
            TrustedStateChange::Epoch {
                new_state,
                latest_epoch_change_li,
            } => {
                println!(
                    "Verified epoch changed to {}",
                    latest_epoch_change_li
                        .ledger_info()
                        .next_epoch_state()
                        .expect("no validator set in epoch change ledger info"),
                );
                // Update client state
                self.trusted_state = Option::from(new_state);
                self.latest_epoch_change_li = Some(latest_epoch_change_li.clone());
            }
            TrustedStateChange::Version { new_state } => {
                if self.trusted_state.as_mut().unwrap().latest_version() < new_state.latest_version() {
                    println!("Verified version change to: {}", new_state.latest_version());
                }
                self.trusted_state = Option::from(new_state);
            }
            TrustedStateChange::NoChange => (),
        }
        Ok(())
    }

    pub fn get_transactions(
        &mut self,
        start_version: u64,
        limit: u64,
        include_events: bool
    ) -> Result<()> {
        let mut batch = JsonRpcBatch::new();
        batch.add_get_transactions_request(start_version, limit, include_events);
        let responses = self.rpc_client.execute(batch).unwrap();
        //println!("response:{:?}", responses);
        let resp = get_response_from_batch(0, &responses).unwrap().as_ref().unwrap();
        self.transactions = Option::from(TransactionView::vec_from_response(resp.clone()).unwrap());
        let transactions= self.transactions.as_ref().unwrap().clone();
        for transaction in transactions {
            println!("transaction version:{:?}, transaction hash:{:?}", transaction.version, transaction.hash);
            match transaction.transaction {
                TransactionDataView::UserTransaction { .. } => {
                    //println!("sender:\n{:?}", sender);
                    println!("transaction:\n{:?}", transaction);
                },
                TransactionDataView::BlockMetadata { timestamp_usecs} => {
                    //println!("transaction:\n{:?}", transaction);
                    println!("BlockMetadata");
                }
                TransactionDataView::WriteSet { } => {
                    println!("WriteSet");
                }
                TransactionDataView::UnknownTransaction { } => {
                    println!("UnknownTransaction");
                }
            }
        }
        Ok(())
    }
}

async fn bridge(args: Args) {
    //official endpoint: https://testnet.diem.com/v1
    let mut demo = LibraDemo::new(&args.diem_rpc_endpoint).unwrap();

    let known_version = 0;
    let mut start: u64 = 0;
    let mut limit: u64 = 100;
    let new_limit: u64 =1;
    loop {
        let _ = demo.init_state(known_version);
        let new_version = demo.trusted_state.as_ref().unwrap().latest_version();
        let end = new_version / limit;
        for index in start..end {
            let _ = demo.get_transactions(index * limit + 1, limit, true);
            if index > 0 && index % 100 == 0 {
                delay_for(Duration::from_millis(300)).await;
            }
        }

        start = end * limit / new_limit;
        limit = new_limit;

        println!("waiting for new versions...");
        delay_for(Duration::from_millis(5000)).await;
    }

}

#[tokio::main]
async fn main() {
    let args = Args::from_args();
    bridge(args).await;
}