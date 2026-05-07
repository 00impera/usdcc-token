use near_sdk::store::LookupMap;
use near_sdk::json_types::U128;
use near_sdk::{
    env, near, near_bindgen, AccountId, NearToken, PanicOnDefault, Promise,
    Gas, PromiseOrValue, ext_contract,
};
use near_sdk::serde::{Deserialize, Serialize};

const GAS_FOR_FT_ON_TRANSFER: Gas = Gas::from_tgas(30);
const GAS_FOR_RESOLVE: Gas = Gas::from_tgas(5);

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct FungibleTokenMetadata {
    pub spec: String, pub name: String, pub symbol: String,
    pub icon: Option<String>, pub reference: Option<String>,
    pub reference_hash: Option<String>, pub decimals: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct StorageBalance { pub total: U128, pub available: U128 }

#[ext_contract(ext_ft_receiver)]
trait FtReceiver {
    fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128>;
}

#[ext_contract(ext_self)]
trait SelfCallback {
    fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128;
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    pub owner_id: AccountId,
    pub total_supply: u128,
    pub balances: LookupMap<AccountId, u128>,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(owner_id: AccountId, total_supply: U128) -> Self {
        let mut balances = LookupMap::new(b"b");
        balances.insert(owner_id.clone(), total_supply.0);
        Self { owner_id, total_supply: total_supply.0, balances }
    }

    #[init(ignore_state)]
    pub fn migrate() -> Self {
        Self {
            owner_id: env::current_account_id(),
            total_supply: 0,
            balances: LookupMap::new(b"b"),
        }
    }

    pub fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: "ft-1.0.0".to_string(),
            name: "USD Coin Cross-Chain".to_string(),
            symbol: "USDCC".to_string(),
            icon: Some("https://files.catbox.moe/ujzf30.gif".to_string()),
            reference: None, reference_hash: None, decimals: 6,
        }
    }

    pub fn ft_total_supply(&self) -> U128 { U128(self.total_supply) }

    pub fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        U128(*self.balances.get(&account_id).unwrap_or(&0))
    }

    #[payable]
    pub fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        let _ = memo;
        let sender = env::predecessor_account_id();
        self.internal_transfer(&sender, &receiver_id, amount.0);
    }

    #[payable]
    pub fn ft_transfer_call(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>, msg: String) -> Promise {
        let _ = memo;
        let sender = env::predecessor_account_id();
        self.internal_transfer(&sender, &receiver_id, amount.0);
        ext_ft_receiver::ext(receiver_id.clone())
            .with_static_gas(GAS_FOR_FT_ON_TRANSFER)
            .ft_on_transfer(sender.clone(), amount, msg)
            .then(
                ext_self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_RESOLVE)
                    .ft_resolve_transfer(sender, receiver_id, amount)
            )
    }

    #[private]
    pub fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128 {
        let unused = match env::promise_result(0) {
            near_sdk::PromiseResult::Successful(value) => {
                if let Ok(u) = near_sdk::serde_json::from_slice::<U128>(&value) {
                    std::cmp::min(amount.0, u.0)
                } else { 0 }
            }
            _ => amount.0,
        };
        if unused > 0 {
            let rb = *self.balances.get(&receiver_id).unwrap_or(&0);
            let refund = std::cmp::min(rb, unused);
            if refund > 0 {
                self.balances.insert(receiver_id, rb - refund);
                let sb = *self.balances.get(&sender_id).unwrap_or(&0);
                self.balances.insert(sender_id, sb + refund);
            }
        }
        U128(amount.0 - unused)
    }

    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        assert_eq!(env::predecessor_account_id(), self.owner_id, "Owner only");
        let bal = *self.balances.get(&account_id).unwrap_or(&0);
        self.balances.insert(account_id, bal + amount.0);
        self.total_supply += amount.0;
    }

    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>) -> StorageBalance {
        let account = account_id.unwrap_or_else(|| env::predecessor_account_id());
        if self.balances.get(&account).is_none() {
            self.balances.insert(account, 0);
        }
        StorageBalance { total: U128(1), available: U128(1) }
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        if self.balances.get(&account_id).is_some() {
            Some(StorageBalance { total: U128(1), available: U128(1) })
        } else { None }
    }

    pub fn burn_and_bridge(&mut self, amount: U128, monad_recipient: String) {
        let caller = env::predecessor_account_id();
        let bal = *self.balances.get(&caller).unwrap_or(&0);
        assert!(bal >= amount.0, "Insufficient balance");
        self.balances.insert(caller, bal - amount.0);
        self.total_supply -= amount.0;
        let bridge: AccountId = "monad-bridge.gemsrock-nft.near".parse().unwrap();
        let args = format!(r#"{{"amount":"{}","monad_recipient":"{}"}}"#, amount.0, monad_recipient);
        Promise::new(bridge).function_call(
            "emit_bridge".to_string(),
            args.into_bytes(),
            NearToken::from_yoctonear(0),
            Gas::from_tgas(10),
        );
    }

    fn internal_transfer(&mut self, sender: &AccountId, receiver: &AccountId, amount: u128) {
        assert!(amount > 0, "Amount must be > 0");
        let sb = *self.balances.get(sender).unwrap_or(&0);
        assert!(sb >= amount, "Insufficient balance");
        self.balances.insert(sender.clone(), sb - amount);
        let rb = *self.balances.get(receiver).unwrap_or(&0);
        self.balances.insert(receiver.clone(), rb + amount);
    }
}
