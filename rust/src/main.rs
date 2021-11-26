/**
* Module     : main.rs
* Copyright  : 2021 DFinance Team
* License    : Apache 2.0 with LLVM Exception
* Maintainer : DFinance Team <hello@dfinance.ai>
* Stability  : Experimental
*/
use candid::{candid_method, CandidType, Deserialize, Int, Nat};
use cap_sdk::{handshake, insert, Event, IndefiniteEvent, TypedEvent};
use cap_std::dip20::cap::DIP20Details;
use cap_std::dip20::{Operation, TransactionStatus, TxRecord};
use ic_kit::{ic , Principal};
use ic_cdk_macros::*;
use ic_types::{CanisterId, PrincipalId};
use ledger_canister::{Memo, icpts::ICPTs, TransactionNotification, account_identifier::AccountIdentifier, SendArgs};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::convert::Into;
use std::iter::FromIterator;
use std::string::String;

#[derive(CandidType, Default, Deserialize)]
pub struct TxLog {
    pub ie_records: VecDeque<IndefiniteEvent>,
}

pub fn tx_log<'a>() -> &'a mut TxLog {
    ic_kit::ic::get_mut::<TxLog>()
}

#[derive(Deserialize, CandidType, Clone, Debug)]
struct Metadata {
    logo: String,
    name: String,
    symbol: String,
    decimals: u8,
    total_supply: Nat,
    owner: Principal,
    fee: Nat,
    fee_to: Principal,
}

#[derive(Deserialize, CandidType, Clone, Debug)]
struct TokenInfo {
    metadata: Metadata,
    fee_to: Principal,
    // status info
    history_size: usize,
    deploy_time: u64,
    holder_number: usize,
    cycles: u64,
}

impl Default for Metadata {
    fn default() -> Self {
        Metadata {
            logo: "".to_string(),
            name: "".to_string(),
            symbol: "".to_string(),
            decimals: 0u8,
            total_supply: Nat::from(0),
            owner: Principal::anonymous(),
            fee: Nat::from(0),
            fee_to: Principal::anonymous(),
        }
    }
}

type Balances = HashMap<Principal, Nat>;
type Allowances = HashMap<Principal, HashMap<Principal, Nat>>;

#[derive(CandidType, Debug, PartialEq)]
pub enum TxError {
    InsufficientBalance,
    InsufficientAllowance,
    Unauthorized,
    LedgerTrap,
    AmountTooSmall,
    Other,
}
type TxReceipt = Result<usize, TxError>;

const LEDGER_CANISTER_ID: CanisterId = CanisterId::from_u64(2);
const THRESHOLD: ICPTs = ICPTs::from_e8s(0); // 0;
const ICPFEE: ICPTs = ICPTs::from_e8s(10000);

#[init]
#[candid_method(init)]
fn init(
    logo: String,
    name: String,
    symbol: String,
    decimals: u8,
    owner: Principal,
    fee: Nat,
    cap: Principal,
) {
    let metadata = ic::get_mut::<Metadata>();
    metadata.logo = logo;
    metadata.name = name;
    metadata.symbol = symbol;
    metadata.decimals = decimals;
    metadata.total_supply = Nat::from(0);
    metadata.owner = owner;
    metadata.fee = fee;
    handshake(1_000_000, Some(cap));
    let _ = add_record(
        Some(owner),
        Operation::Mint,
        Principal::from_text("aaaaa-aa").unwrap(),
        owner,
        Nat::from(0),
        Nat::from(0),
        ic::time(),
        TransactionStatus::Succeeded,
    );
}

fn _transfer(from: Principal, to: Principal, value: Nat) {
    let balances = ic::get_mut::<Balances>();
    let from_balance = balance_of(from);
    let from_balance_new = from_balance - value.clone();
    if from_balance_new != 0 {
        balances.insert(from, from_balance_new);
    } else {
        balances.remove(&from);
    }
    let to_balance = balance_of(to);
    let to_balance_new = to_balance + value;
    if to_balance_new != 0 {
        balances.insert(to, to_balance_new);
    }
}

fn _charge_fee(user: Principal, fee_to: Principal, fee: Nat) {
    let metadata = ic::get::<Metadata>();
    if metadata.fee > Nat::from(0) {
        _transfer(user, fee_to, fee);
    }
}

#[update(name = "transfer")]
#[candid_method(update)]
async fn transfer(to: Principal, value: Nat) -> TxReceipt {
    let from = ic::caller();
    let metadata = ic::get::<Metadata>();
    if balance_of(from) < value.clone() + metadata.fee.clone() {
        return Err(TxError::InsufficientBalance);
    }
    _charge_fee(from, metadata.fee_to, metadata.fee.clone());
    _transfer(from, to, value.clone());
    
    add_record(
        None,
        Operation::Transfer,
        from,
        to,
        value,
        metadata.fee.clone(),
        ic::time(),
        TransactionStatus::Succeeded,
    )
    .await
}

#[update(name = "transferFrom")]
#[candid_method(update, rename = "transferFrom")]
async fn transfer_from(from: Principal, to: Principal, value: Nat) -> TxReceipt {
    let owner = ic::caller();
    let from_allowance = allowance(from, owner);
    let metadata = ic::get::<Metadata>();
    if from_allowance < value.clone() + metadata.fee.clone() {
        return Err(TxError::InsufficientAllowance);
    } 
    let from_balance = balance_of(from);
    if from_balance < value.clone() + metadata.fee.clone() {
        return Err(TxError::InsufficientBalance);
    }
    _charge_fee(from, metadata.fee_to, metadata.fee.clone());
    _transfer(from, to, value.clone());
    let allowances = ic::get_mut::<Allowances>();
    match allowances.get(&from) {
        Some(inner) => {
            let result = inner.get(&owner).unwrap().clone();
            let mut temp = inner.clone();
            if result.clone() - value.clone() - metadata.fee.clone() != 0 {
                temp.insert(owner, result - value.clone() - metadata.fee.clone());
                allowances.insert(from, temp);
            } else {
                temp.remove(&owner);
                if temp.len() == 0 {
                    allowances.remove(&from);
                } else {
                    allowances.insert(from, temp);
                }
            }
        }
        None => {
            assert!(false);
        }
    }
    add_record(
        Some(owner),
        Operation::TransferFrom,
        from,
        to,
        value,
        metadata.fee.clone(),
        ic::time(),
        TransactionStatus::Succeeded,
    )
    .await
}

#[update(name = "approve")]
#[candid_method(update)]
async fn approve(spender: Principal, value: Nat) -> TxReceipt {
    let owner = ic::caller();
    let metadata = ic::get::<Metadata>();
    if balance_of(owner) < metadata.fee.clone() {
        return Err(TxError::InsufficientBalance);
    }
    _charge_fee(owner, metadata.fee_to, metadata.fee.clone());
    let v = value.clone() + metadata.fee.clone();
    let allowances = ic::get_mut::<Allowances>();
    match allowances.get(&owner) {
        Some(inner) => {
            let mut temp = inner.clone();
            if v.clone() != 0 {
                temp.insert(spender, v.clone());
                allowances.insert(owner, temp);
            } else {
                temp.remove(&spender);
                if temp.len() == 0 {
                    allowances.remove(&owner);
                } else {
                    allowances.insert(owner, temp);
                }
            }
        }
        None => {
            if v.clone() != 0 {
                let mut inner = HashMap::new();
                inner.insert(spender, v.clone());
                let allowances = ic::get_mut::<Allowances>();
                allowances.insert(owner, inner);
            }
        }
    }
    add_record(
        None,
        Operation::Approve,
        owner,
        spender,
        v,
        metadata.fee.clone(),
        ic::time(),
        TransactionStatus::Succeeded,
    )
    .await
}

#[update(name = "transaction_notification")]
#[candid_method(update, rename = "transaction_notification")]
async fn transaction_notification(tn: TransactionNotification) -> TxReceipt {
    let caller = ic::caller();
    let caller_p = PrincipalId::from(caller);
    if CanisterId::new(caller_p) != Ok(LEDGER_CANISTER_ID) {
        return Err(TxError::Unauthorized);
    }

    if tn.amount < THRESHOLD {
        return Err(TxError::AmountTooSmall);
    }

    if CanisterId::get(tn.to).0 != ic::id() {
        return Err(TxError::Unauthorized);
    }

    let user = tn.from.0;
    let value = Nat::from(ICPTs::get_e8s(tn.amount));

    let user_balance = balance_of(user);
    let balances = ic::get_mut::<Balances>();
    balances.insert(user, user_balance + value.clone());
    let metadata = ic::get_mut::<Metadata>();
    metadata.total_supply += value.clone();
    
    add_record(
        Some(caller),
        Operation::Mint,
        user,
        user,
        value,
        Nat::from(0),
        ic::time(),
        TransactionStatus::Succeeded,
    )
    .await
}

#[update(name = "withdraw")]
#[candid_method(update, rename = "withdraw")]
async fn withdraw(value: u64, to: String) -> TxReceipt {
    if ICPTs::from_e8s(value) < THRESHOLD {
        return Err(TxError::AmountTooSmall);
    }
    let caller = ic::caller();
    let caller_balance = balance_of(caller);
    let value_nat = Nat::from(value);
    let metadata = ic::get_mut::<Metadata>();
    if caller_balance.clone() < value_nat.clone() || metadata.total_supply < value_nat.clone() {
        return Err(TxError::InsufficientBalance);
    }
    let args = SendArgs {
        memo: Memo(0x57444857),
        amount: (ICPTs::from_e8s(value) - ICPFEE).unwrap(),
        fee: ICPFEE,
        from_subaccount: None,
        to: AccountIdentifier::from_hex(&to).unwrap(),
        created_at_time: None,
    };
    let balances = ic::get_mut::<Balances>();
    balances.insert(caller, caller_balance.clone() - value_nat.clone());
    metadata.total_supply -= value_nat.clone();
    let result: Result<(u64,), _> = ic::call(Principal::from(CanisterId::get(LEDGER_CANISTER_ID)), "send_dfx", (args,)).await;
    match result {
        Ok(_) => {
            add_record(
                None,
                Operation::Burn,
                caller,
                caller,
                value_nat,
                Nat::from(0),
                ic::time(),
                TransactionStatus::Succeeded,
            )
            .await
        },
        Err(_) => {
            balances.insert(caller, caller_balance);
            metadata.total_supply += value_nat;
            return Err(TxError::LedgerTrap);
        },
    }
}

#[update(name = "setLogo")]
#[candid_method(update, rename = "setLogo")]
fn set_logo(logo: String) {
    let metadata = ic::get_mut::<Metadata>();
    assert_eq!(ic::caller(), metadata.owner);
    metadata.logo = logo;
}

#[update(name = "setFee")]
#[candid_method(update, rename = "setFee")]
fn set_fee(fee: Nat) {
    let metadata = ic::get_mut::<Metadata>();
    assert_eq!(ic::caller(), metadata.owner);
    metadata.fee = fee;
}

#[update(name = "setFeeTo")]
#[candid_method(update, rename = "setFeeTo")]
fn set_fee_to(fee_to: Principal) {
    let metadata = ic::get_mut::<Metadata>();
    assert_eq!(ic::caller(), metadata.owner);
    metadata.fee_to = fee_to;
}

#[update(name = "setOwner")]
#[candid_method(update, rename = "setOwner")]
fn set_owner(owner: Principal) {
    let metadata = ic::get_mut::<Metadata>();
    assert_eq!(ic::caller(), metadata.owner);
    metadata.owner = owner;
}

#[query(name = "balanceOf")]
#[candid_method(query, rename = "balanceOf")]
fn balance_of(id: Principal) -> Nat {
    let balances = ic::get::<Balances>();
    match balances.get(&id) {
        Some(balance) => balance.clone(),
        None => Nat::from(0),
    }
}

#[query(name = "allowance")]
#[candid_method(query)]
fn allowance(owner: Principal, spender: Principal) -> Nat {
    let allowances = ic::get::<Allowances>();
    match allowances.get(&owner) {
        Some(inner) => match inner.get(&spender) {
            Some(value) => value.clone(),
            None => Nat::from(0),
        },
        None => Nat::from(0),
    }
}

#[query(name = "getLogo")]
#[candid_method(query, rename = "getLogo")]
fn get_logo() -> String {
    let metadata = ic::get::<Metadata>();
    metadata.logo.clone()
}

#[query(name = "name")]
#[candid_method(query)]
fn name() -> String {
    let metadata = ic::get::<Metadata>();
    metadata.name.clone()
}

#[query(name = "symbol")]
#[candid_method(query)]
fn symbol() -> String {
    let metadata = ic::get::<Metadata>();
    metadata.symbol.clone()
}

#[query(name = "decimals")]
#[candid_method(query)]
fn decimals() -> u8 {
    let metadata = ic::get::<Metadata>();
    metadata.decimals
}

#[query(name = "totalSupply")]
#[candid_method(query, rename = "totalSupply")]
fn total_supply() -> Nat {
    let metadata = ic::get::<Metadata>();
    metadata.total_supply.clone()
}

#[query(name = "owner")]
#[candid_method(query)]
fn owner() -> Principal {
    let metadata = ic::get::<Metadata>();
    metadata.owner
}

#[query(name = "getMetadta")]
#[candid_method(query, rename = "getMetadta")]
fn get_metadata() -> Metadata {
    ic::get::<Metadata>().clone()
}


#[query(name = "historySize")]
#[candid_method(query, rename = "historySize")]
fn history_size() -> usize {
    // history handling needs fixing after CAP SDK is ready
    unimplemented!()
}

#[update(name = "getTransaction")]
#[candid_method(update, rename = "getTransaction")]
async fn get_transaction(_index: usize) -> TxRecord {
    // let res = cap_sdk::get_transaction(_index as u64)
    //     .await
    //     .expect("unable to retrieve transaction from CAP");
    // ic_cdk::print(format!("{:?}", res));
    // OpRecord{
    //     caller: Some(Principal::anonymous()),
    //     op: Operation::Mint,
    //     index: 0,
    //     from: Principal::anonymous(),
    //     to: Principal::anonymous(),
    //     amount: 1,
    //     fee: 2,
    //     timestamp: 3,
    //     status: TransactionStatus::Succeeded,
    //}

    // history handling needs fixing after CAP SDK is ready
    unimplemented!();
}

#[query(name = "getTransactions")]
#[candid_method(query, rename = "getTransactions")]
fn get_transactions(_start: usize, _limit: usize) -> Vec<TxRecord> {
    // history handling needs fixing after CAP SDK is ready
    unimplemented!()
}

#[query(name = "getUserTransactionAmount")]
#[candid_method(query, rename = "getUserTransactionAmount")]
fn get_user_transaction_amount(_user: Principal) -> usize {
    // history handling needs fixing after CAP SDK is ready
    unimplemented!()
}

#[query(name = "getUserTransactions")]
#[candid_method(query, rename = "getUserTransactions")]
fn get_user_transactions(_user: Principal, _start: usize, _limit: usize) -> Vec<TxRecord> {
    // history handling needs fixing after CAP SDK is ready
    unimplemented!()
}

#[query(name = "getTokenInfo")]
#[candid_method(query, rename = "getTokenInfo")]
fn get_token_info() -> TokenInfo {
    let metadata = ic::get::<Metadata>().clone();
    let balance = ic::get::<Balances>();

    return TokenInfo {
        metadata: metadata.clone(),
        fee_to: metadata.fee_to,
        history_size: 0, // history handling needs fixing after CAP SDK is ready
        deploy_time: 0,  // history handling needs fixing after CAP SDK is ready,
        holder_number: balance.len(),
        cycles: ic::balance(),
    };
}

#[query(name = "getHolders")]
#[candid_method(query, rename = "getHolders")]
fn get_holders(start: usize, limit: usize) -> Vec<(Principal, Nat)> {
    let mut balance = Vec::new();
    for (k, v) in ic::get::<Balances>().clone() {
        balance.push((k, v.clone()));
    }
    balance.sort_by(|a, b| b.1.cmp(&a.1));
    let limit: usize = if start + limit > balance.len() {
        balance.len() - start
    } else {
        limit
    };
    balance[start..start + limit].to_vec()
}

#[query(name = "getAllowanceSize")]
#[candid_method(query, rename = "getAllowanceSize")]
fn get_allowance_size() -> usize {
    let mut size = 0;
    let allowances = ic::get::<Allowances>();
    for (_, v) in allowances.iter() {
        size += v.len();
    }
    size
}

#[query(name = "getUserApprovals")]
#[candid_method(query, rename = "getUserApprovals")]
fn get_user_approvals(who: Principal) -> Vec<(Principal, Nat)> {
    let allowances = ic::get::<Allowances>();
    match allowances.get(&who) {
        Some(allow) => return Vec::from_iter(allow.clone().into_iter()),
        None => return Vec::new(),
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn main() {}

#[cfg(not(any(target_arch = "wasm32", test)))]
fn main() {
    candid::export_service!();
    std::print!("{}", __export_service());
}

// TODO: fix upgrade functions
#[pre_upgrade]
fn pre_upgrade() {
    ic::stable_store((ic::get::<Metadata>().clone(),ic::get::<Balances>(), ic::get::<Allowances>(), tx_log())).unwrap();
}

#[post_upgrade]
fn post_upgrade() {
    let (metadata_stored, balances_stored, allowances_stored, tx_log_stored): (Metadata,Balances,Allowances,TxLog) = ic::stable_restore().unwrap();
    let metadata = ic::get_mut::<Metadata>();
    *metadata = metadata_stored;

    let balances = ic::get_mut::<Balances>();
    *balances = balances_stored;

    let allowances = ic::get_mut::<Allowances>();
    *allowances = allowances_stored;

    let tx_log = tx_log();
    *tx_log = tx_log_stored;
}


async fn add_record(
    caller: Option<Principal>,
    op: Operation,
    from: Principal,
    to: Principal,
    amount: Nat,
    fee: Nat,
    timestamp: u64,
    status: TransactionStatus,
) -> TxReceipt {
    insert_into_cap(Into::<IndefiniteEvent>::into(Into::<Event>::into(Into::<
        TypedEvent<DIP20Details>,
    >::into(
        TxRecord {
            caller,
            index: Nat::from(0),
            from,
            to,
            amount: Nat::from(amount),
            fee: Nat::from(fee),
            timestamp: Int::from(timestamp),
            status,
            operation: op,
        },
    ))))
    .await
}

pub async fn insert_into_cap(ie: IndefiniteEvent) -> TxReceipt {
    let tx_log = tx_log();
    if let Some(failed_ie) = tx_log.ie_records.pop_front() {
        let _ = insert_into_cap_priv(failed_ie).await;
    }
    insert_into_cap_priv(ie).await
}

async fn insert_into_cap_priv(ie: IndefiniteEvent) -> TxReceipt {
    let insert_res = insert(ie.clone())
        .await
        .map(|tx_id| tx_id as usize)
        .map_err(|_| TxError::Other);

    if insert_res.is_err() {
        tx_log().ie_records.push_back(ie.clone());
    }

    insert_res
}

#[cfg(test)]
mod tests {
    use super::*;
    use ic_kit::{mock_principals::{alice, bob, john}, MockContext};
    use assert_panic::assert_panic;

    fn initialize_tests() {
      init(
        String::from("logo"),
        String::from("token"),
        String::from("TOKEN"),
        2,
        1_000,
        alice(),
        1,
      );
    }

    #[test]
    fn functionality_test() {
      MockContext::new()
      .with_balance(100_000)
      .with_caller(alice())
      .inject();

      initialize_tests();

      // initialization tests
      assert_eq!(balance_of(alice()), 1_000, "balanceOf did not return the correct value");
      assert_eq!(total_supply(), 1_000, "totalSupply did not return the correct value");
      assert_eq!(symbol(), String::from("TOKEN"), "symbol did not return the correct value");
      assert_eq!(owner(), alice(), "owner did not return the correct value");
      assert_eq!(name(), String::from("token"), "name did not return the correct value");
      assert_eq!(get_logo(), String::from("logo"), "getLogo did not return the correct value");
      assert_eq!(decimals(), 2, "decimals did not return the correct value");
      assert_eq!(get_holders(0, 10).len(), 1, "get_holders returned the correct amount of holders after initialization");
      assert_eq!(get_transaction(0).op, Operation::Mint, "get_transaction returnded a Mint operation");

      let token_info = get_token_info();
      assert_eq!(token_info.fee_to, Principal::anonymous(), "tokenInfo.fee_to did not return the correct value");
      assert_eq!(token_info.history_size, 1, "tokenInfo.history_size did not return the correct value");
      assert!(token_info.deploy_time > 0, "tokenInfo.deploy_time did not return the correct value");
      assert_eq!(token_info.holder_number, 1, "tokenInfo.holder_number did not return the correct value");
      assert_eq!(token_info.cycles, 100_000, "tokenInfo.cycles did not return the correct value");

      let metadata = get_metadata();
      assert_eq!(metadata.total_supply, 1_000, "metadata.total_supply did not return the correct value");
      assert_eq!(metadata.symbol, String::from("TOKEN"), "metadata.symbol did not return the correct value");
      // assert_eq!(metadata.owner, alice(), "metadata.owner did not return the correct value");
      assert_eq!(metadata.name, String::from("token"), "metadata.name did not return the correct value");
      assert_eq!(metadata.logo, String::from("logo"), "metadata.logo did not return the correct value");
      assert_eq!(metadata.decimals, 2, "metadata.decimals did not return the correct value");
      assert_eq!(metadata.fee, 1, "metadata.fee did not return the correct value");
      assert_eq!(metadata.fee_to, Principal::anonymous(), "metadata.fee_to did not return the correct value");

      // set fee test
      set_fee(2);
      assert_eq!(2, get_metadata().fee ,"Failed to update the fee_to");

      // set fee_to test
      set_fee_to(john());
      assert_eq!(john(), get_metadata().fee_to, "Failed to set fee");
      set_fee_to(Principal::anonymous());

      // set logo
      set_logo(String::from("new_logo"));
      assert_eq!("new_logo", get_logo());

      // test transfers
      let transfer_alice_balance_expected = balance_of(alice()) - 10 - get_metadata().fee;
      let transfer_bob_balance_expected = balance_of(bob()) + 10;
      let transfer_john_balance_expected = balance_of(john());
      let transfer_transaction_amount_expected = get_transactions(0, 10).len() + 1;
      let transfer_user_transaction_amount_expected = get_user_transaction_amount(alice()) + 1;
      transfer(bob(), 10).map_err(|err| println!("{:?}", err)).ok();

      assert_eq!(balance_of(alice()), transfer_alice_balance_expected, "Transfer did not transfer the expected amount to Alice");
      assert_eq!(balance_of(bob()), transfer_bob_balance_expected, "Transfer did not transfer the expected amount to Bob");
      assert_eq!(balance_of(john()), transfer_john_balance_expected, "Transfer did not transfer the expected amount to John");
      assert_eq!(get_transactions(0, 10).len(), transfer_transaction_amount_expected, "transfer operation did not produce a transaction");
      assert_eq!(get_user_transaction_amount(alice()), transfer_user_transaction_amount_expected, "get_user_transaction_amount returned the wrong value after a transfer");
      assert_eq!(get_user_transactions(alice(), 0, 10).len(), transfer_user_transaction_amount_expected, "get_user_transactions returned the wrong value after a transfer");
      assert_eq!(get_holders(0, 10).len(), 3, "get_holders returned the correct amount of holders after transfer");
      assert_eq!(get_transaction(1).op, Operation::Transfer, "get_transaction returnded a Transfer operation");

      // test allowances
      approve(bob(), 100).map_err(|err| println!("{:?}", err)).ok();
      assert_eq!(allowance(alice(), bob()), 100 + get_metadata().fee, "Approve did not give the correct allowance");
      assert_eq!(get_allowance_size(), 1, "getAllowanceSize returns the correct value");
      assert_eq!(get_user_approvals(alice()).len(), 1, "getUserApprovals not returning the correct value");

      // test transfer_from
      // inserting an allowance of Alice for Bob's balance to test transfer_from
      let allowances = ic::get_mut::<Allowances>();
      let mut inner = HashMap::new();
      inner.insert(alice(), 5 + get_metadata().fee);
      allowances.insert(bob(), inner);

      let transfer_from_alice_balance_expected = balance_of(alice());
      let transfer_from_bob_balance_expected = balance_of(bob()) - 5 - get_metadata().fee;
      let transfer_from_john_balance_expected = balance_of(john()) + 5;
      let transfer_from_transaction_amount_expected = get_transactions(0, 10).len() + 1;

      transfer_from(bob(), john(), 5).map_err(|err| println!("{:?}", err)).ok();

      assert_eq!(balance_of(alice()), transfer_from_alice_balance_expected, "transfer_from transferred the correct value for alice");
      assert_eq!(balance_of(bob()), transfer_from_bob_balance_expected, "transfer_from transferred the correct value for bob");
      assert_eq!(balance_of(john()), transfer_from_john_balance_expected, "transfer_from transferred the correct value for john");
      assert_eq!(allowance(bob(), alice()), 0, "allowance has not been spent");
      assert_eq!(get_transactions(0, 10).len(), transfer_from_transaction_amount_expected, "transfer_from operation did not produce a transaction");

      // Transferring more than the balance
      assert_eq!(transfer(alice(), 1_000_000), Err(TxError::InsufficientBalance) , "alice was able to transfer more than is allowed");
      // Transferring more than the balance
      assert_eq!(transfer_from(bob(), john(), 1_000_000), Err(TxError::InsufficientAllowance) , "alice was able to transfer more than is allowed");

      //set owner test
      set_owner(bob());
      assert_eq!(bob(), owner(), "Failed to set new owner");
    }

    #[test]
    fn permission_tests() {
      MockContext::new()
      .with_balance(100_000)
      .with_caller(bob())
      .inject();

      initialize_tests();

      assert_panic!(set_logo(String::from("forbidden")));
      assert_panic!(set_fee(123));
      assert_panic!(set_fee_to(john()));
      assert_panic!(set_owner(bob()));
    }
}
