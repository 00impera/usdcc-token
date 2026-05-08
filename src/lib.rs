use near_sdk::store::LookupMap;
use near_sdk::json_types::U128;
use near_sdk::{
    env, near_bindgen, AccountId, NearToken, PanicOnDefault, Promise,
    PromiseOrValue, Gas, BorshStorageKey,
};
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::serde_json;

const GAS_FOR_FT_TRANSFER_CALL: Gas = Gas::from_tgas(35);
const GAS_FOR_RESOLVE: Gas = Gas::from_tgas(10);

#[derive(BorshStorageKey, BorshSerialize)]
enum StorageKey { Accounts, StorageDeposits }

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct FungibleTokenMetadata {
    pub spec: String,
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub reference: Option<String>,
    pub reference_hash: Option<String>,
    pub decimals: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct StorageBalance {
    pub total: U128,
    pub available: U128,
}

#[derive(BorshDeserialize, BorshSerialize)]
pub struct StoredMetadata {
    pub name: String,
    pub symbol: String,
    pub icon: Option<String>,
    pub decimals: u8,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub owner_id: AccountId,
    pub total_supply: u128,
    pub accounts: LookupMap<AccountId, u128>,
    pub storage_deposits: LookupMap<AccountId, u128>,
    pub metadata: StoredMetadata,
}

#[near_bindgen]
impl Contract {
    #[init]
    pub fn new(owner_id: AccountId, total_supply: U128, name: String, symbol: String, icon: Option<String>, decimals: u8) -> Self {
        let mut accounts = LookupMap::new(StorageKey::Accounts);
        accounts.insert(owner_id.clone(), total_supply.0);
        Self {
            owner_id,
            total_supply: total_supply.0,
            accounts,
            storage_deposits: LookupMap::new(StorageKey::StorageDeposits),
            metadata: StoredMetadata { name, symbol, icon, decimals },
        }
    }

    #[init]
    pub fn new_default_meta(owner_id: AccountId, total_supply: U128) -> Self {
        Self::new(
            owner_id,
            total_supply,
            "USD Coin Cross-Chain".to_string(),
            "USDCC".to_string(),
            Some("https://files.catbox.moe/ujzf30.gif".to_string()),
            9,
        )
    }

    pub fn ft_metadata(&self) -> FungibleTokenMetadata {
        FungibleTokenMetadata {
            spec: "ft-1.0.0".to_string(),
            name: self.metadata.name.clone(),
            symbol: self.metadata.symbol.clone(),
            icon: self.metadata.icon.clone(),
            reference: None,
            reference_hash: None,
            decimals: self.metadata.decimals,
        }
    }

    pub fn ft_total_supply(&self) -> U128 { U128(self.total_supply) }

    pub fn ft_balance_of(&self, account_id: AccountId) -> U128 {
        U128(self.accounts.get(&account_id).copied().unwrap_or(0))
    }

    #[payable]
    pub fn ft_transfer(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>) {
        assert_eq!(env::attached_deposit().as_yoctonear(), 1, "Requires 1 yoctoNEAR");
        let _ = memo;
        self.internal_transfer(&env::predecessor_account_id(), &receiver_id, amount.0);
    }

    #[payable]
    pub fn ft_transfer_call(&mut self, receiver_id: AccountId, amount: U128, memo: Option<String>, msg: String) -> Promise {
        assert_eq!(env::attached_deposit().as_yoctonear(), 1, "Requires 1 yoctoNEAR");
        let _ = memo;
        let sender_id = env::predecessor_account_id();
        self.internal_transfer(&sender_id, &receiver_id, amount.0);
        Promise::new(receiver_id.clone())
            .function_call(
                "ft_on_transfer".to_string(),
                serde_json::json!({
                    "sender_id": sender_id,
                    "amount": amount,
                    "msg": msg
                }).to_string().into_bytes(),
                NearToken::from_yoctonear(0),
                GAS_FOR_FT_TRANSFER_CALL,
            )
            .then(
                Promise::new(env::current_account_id())
                    .function_call(
                        "ft_resolve_transfer".to_string(),
                        serde_json::json!({
                            "sender_id": sender_id,
                            "receiver_id": receiver_id,
                            "amount": amount
                        }).to_string().into_bytes(),
                        NearToken::from_yoctonear(0),
                        GAS_FOR_RESOLVE,
                    )
            )
    }

    #[private]
    pub fn ft_resolve_transfer(&mut self, sender_id: AccountId, receiver_id: AccountId, amount: U128) -> U128 {
        let unused_amount = match env::promise_result(0) {
            near_sdk::PromiseResult::Successful(value) => {
                if let Ok(unused) = serde_json::from_slice::<U128>(&value) {
                    std::cmp::min(amount.0, unused.0)
                } else { 0 }
            }
            _ => amount.0,
        };
        if unused_amount > 0 {
            let receiver_balance = self.accounts.get(&receiver_id).copied().unwrap_or(0);
            let refund = std::cmp::min(receiver_balance, unused_amount);
            if refund > 0 {
                self.accounts.insert(receiver_id, receiver_balance - refund);
                let sender_balance = self.accounts.get(&sender_id).copied().unwrap_or(0);
                self.accounts.insert(sender_id, sender_balance + refund);
            }
        }
        U128(amount.0 - unused_amount)
    }

    pub fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128> {
        let _ = (sender_id, msg);
        PromiseOrValue::Value(U128(0))
    }

    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>) -> StorageBalance {
        let amount = env::attached_deposit().as_yoctonear();
        let account = account_id.unwrap_or_else(|| env::predecessor_account_id());
        let current = self.storage_deposits.get(&account).copied().unwrap_or(0);
        self.storage_deposits.insert(account.clone(), current + amount);
        if self.accounts.get(&account).is_none() {
            self.accounts.insert(account, 0);
        }
        StorageBalance { total: U128(current + amount), available: U128(current + amount) }
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> Option<StorageBalance> {
        let total = self.storage_deposits.get(&account_id).copied().unwrap_or(0);
        Some(StorageBalance { total: U128(total), available: U128(total) })
    }

    #[payable]
    pub fn storage_withdraw(&mut self, amount: Option<U128>) -> StorageBalance {
        assert_eq!(env::attached_deposit().as_yoctonear(), 1, "Requires 1 yoctoNEAR");
        let account_id = env::predecessor_account_id();
        let current = self.storage_deposits.get(&account_id).copied().unwrap_or(0);
        let withdraw = amount.map(|a| a.0).unwrap_or(current);
        assert!(withdraw <= current, "Not enough storage balance");
        self.storage_deposits.insert(account_id.clone(), current - withdraw);
        Promise::new(account_id).transfer(NearToken::from_yoctonear(withdraw));
        StorageBalance { total: U128(current - withdraw), available: U128(current - withdraw) }
    }

    pub fn mint(&mut self, account_id: AccountId, amount: U128) {
        assert_eq!(env::predecessor_account_id(), self.owner_id, "Owner only");
        let balance = self.accounts.get(&account_id).copied().unwrap_or(0);
        self.accounts.insert(account_id, balance + amount.0);
        self.total_supply += amount.0;
    }

    #[private]
    pub fn migrate(&mut self) {}

    fn internal_transfer(&mut self, sender_id: &AccountId, receiver_id: &AccountId, amount: u128) {
        assert_ne!(sender_id, receiver_id, "Cannot transfer to self");
        assert!(amount > 0, "Amount must be greater than zero");
        let sender_balance = self.accounts.get(sender_id).copied().unwrap_or(0);
        assert!(sender_balance >= amount, "Not enough balance");
        self.accounts.insert(sender_id.clone(), sender_balance - amount);
        let receiver_balance = self.accounts.get(receiver_id).copied().unwrap_or(0);
        self.accounts.insert(receiver_id.clone(), receiver_balance + amount);
    }
}
