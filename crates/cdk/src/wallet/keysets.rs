use tracing::instrument;

use crate::nuts::{Id, KeySetInfo, Keys};
use crate::{Error, Wallet};

impl Wallet {
    /// Get keys for mint keyset
    ///
    /// Selected keys from localstore if they are already known
    /// If they are not known queries mint for keyset id and stores the [`Keys`]
    #[instrument(skip(self))]
    pub async fn get_keyset_keys(&self, keyset_id: Id) -> Result<Keys, Error> {
        let keys = if let Some(keys) = self.localstore.get_keys(&keyset_id).await? {
            keys
        } else {
            let keys = self
                .client
                .get_mint_keyset(self.mint_url.clone(), keyset_id)
                .await?;

            keys.verify_id()?;

            self.localstore.add_keys(keys.keys.clone()).await?;

            keys.keys
        };

        Ok(keys)
    }

    /// Straight up yolo those keys into the db no cap
    pub async fn add_keyset(
        &self,
        keys: Keys,
        active: bool,
        input_fee_ppk: u64,
    ) -> Result<(), Error> {
        // add to localstore
        self.localstore.add_keys(keys.clone()).await?;

        let keyset_info = KeySetInfo {
            id: Id::from(&keys),
            active,
            unit: self.unit.clone(),
            input_fee_ppk,
        };

        // somehow also add to localstore...why do I need to make two different calls?
        self.localstore
            .add_mint_keysets(self.mint_url.clone(), vec![keyset_info])
            .await?;

        Ok(())
    }

    /// Get keysets for mint
    ///
    /// Queries mint for all keysets
    #[instrument(skip(self))]
    pub async fn get_mint_keysets(&self) -> Result<Vec<KeySetInfo>, Error> {
        let keysets = self.client.get_mint_keysets(self.mint_url.clone()).await?;

        self.localstore
            .add_mint_keysets(self.mint_url.clone(), keysets.keysets.clone())
            .await?;

        Ok(keysets.keysets)
    }

    /// Get active keyset for mint
    ///
    /// Queries mint for current keysets then gets [`Keys`] for any unknown
    /// keysets
    #[instrument(skip(self))]
    pub async fn get_active_mint_keysets(&self) -> Result<Vec<KeySetInfo>, Error> {
        let keysets = self.client.get_mint_keysets(self.mint_url.clone()).await?;
        let keysets = keysets.keysets;

        self.localstore
            .add_mint_keysets(self.mint_url.clone(), keysets.clone())
            .await?;

        let active_keysets = keysets
            .clone()
            .into_iter()
            .filter(|k| k.active && k.unit == self.unit)
            .collect::<Vec<KeySetInfo>>();

        match self
            .localstore
            .get_mint_keysets(self.mint_url.clone())
            .await?
        {
            Some(known_keysets) => {
                let unknown_keysets: Vec<&KeySetInfo> = keysets
                    .iter()
                    .filter(|k| known_keysets.contains(k))
                    .collect();

                for keyset in unknown_keysets {
                    self.get_keyset_keys(keyset.id).await?;
                }
            }
            None => {
                for keyset in keysets {
                    self.get_keyset_keys(keyset.id).await?;
                }
            }
        }

        Ok(active_keysets)
    }

    /// Get active keyset for mint from local without querying the mint
    #[instrument(skip(self))]
    pub async fn get_active_mint_keyset_local(&self) -> Result<KeySetInfo, Error> {
        let active_keysets = match self
            .localstore
            .get_mint_keysets(self.mint_url.clone())
            .await?
        {
            Some(keysets) => keysets
                .into_iter()
                .filter(|k| k.active && k.unit == self.unit)
                .collect::<Vec<KeySetInfo>>(),
            None => {
                vec![]
            }
        };

        let keyset_with_lowest_fee = active_keysets
            .into_iter()
            .min_by_key(|key| key.input_fee_ppk)
            .ok_or(Error::NoActiveKeyset)?;

        Ok(keyset_with_lowest_fee)
    }

    /// Get active keyset for mint with the lowest fees
    ///
    /// Queries mint for current keysets then gets [`Keys`] for any unknown
    /// keysets
    #[instrument(skip(self))]
    pub async fn get_active_mint_keyset(&self) -> Result<KeySetInfo, Error> {
        let active_keysets = self.get_active_mint_keysets().await?;

        let keyset_with_lowest_fee = active_keysets
            .into_iter()
            .min_by_key(|key| key.input_fee_ppk)
            .ok_or(Error::NoActiveKeyset)?;
        Ok(keyset_with_lowest_fee)
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, sync::Arc};

    use crate::cdk_database;
    use crate::cdk_database::WalletMemoryDatabase;
    use crate::nuts;
    use crate::types::QuoteTTL;
    use crate::Mint;
    use crate::Wallet;
    use bip39::Language;
    use bip39::Mnemonic;
    use bitcoin::bip32::{ChildNumber, DerivationPath};
    use cdk_database::mint_memory::MintMemoryDatabase;
    use nuts::{CurrencyUnit, MintInfo, Nuts};
    use rand::Rng;

    // TODO consolidate these constants with the same constants in roles/pool/src/lib/mod.rs
    pub const HASH_CURRENCY_UNIT: &str = "HASH";
    pub const HASH_DERIVATION_PATH: u32 = 1337;

    async fn create_mint() -> Mint {
        let nuts = Nuts::new().nut07(true);

        let mint_info = MintInfo::new().nuts(nuts);

        let entropy = rand::thread_rng().gen::<[u8; 16]>();
        let mnemonic = Mnemonic::from_entropy_in(Language::English, &entropy)
            .expect("Failed to generate mnemonic");

        let hash_currency_unit = CurrencyUnit::Custom(HASH_CURRENCY_UNIT.to_string());

        let mut currency_units = HashMap::new();
        currency_units.insert(hash_currency_unit.clone(), (0, 1));

        let mut derivation_paths = HashMap::new();
        derivation_paths.insert(
            hash_currency_unit,
            DerivationPath::from(vec![
                ChildNumber::from_hardened_idx(0).expect("Failed to create purpose index 0"),
                ChildNumber::from_hardened_idx(HASH_DERIVATION_PATH).unwrap_or_else(|_| {
                    panic!("Failed to create coin type index {}", HASH_DERIVATION_PATH)
                }),
                ChildNumber::from_hardened_idx(0).expect("Failed to create account index 0"),
            ]),
        );

        Mint::new(
            // TODO is mint_url necessary?
            "http://localhost:8000",
            &mnemonic.to_seed_normalized(""),
            mint_info,
            QuoteTTL::new(1000, 1000),
            Arc::new(MintMemoryDatabase::default()),
            HashMap::new(),
            currency_units,
            derivation_paths,
        )
        .await
        .unwrap()
    }

    fn create_wallet() -> Wallet {
        use rand::Rng;

        let seed = rand::thread_rng().gen::<[u8; 32]>();
        let mint_url = "https://testnut.cashu.space";

        let localstore = WalletMemoryDatabase::default();
        Wallet::new(
            mint_url,
            CurrencyUnit::Custom(HASH_CURRENCY_UNIT.to_string()),
            Arc::new(localstore),
            &seed,
            None,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_add_and_get_active_mint_keysets_local() {
        let mint = create_mint().await;
        let keyset_response = mint.keysets().await.unwrap();
        let keyset_info = keyset_response.keysets.first().unwrap();
        let keyset = mint.keyset(&keyset_info.id).await.unwrap().unwrap();

        let wallet = create_wallet();

        // Add the keyset
        wallet.add_keyset(keyset.keys, true, 0).await.unwrap();

        // Retrieve the keysets locally
        let active_keyset = wallet.get_active_mint_keyset_local().await.unwrap();

        // Validate the retrieved keyset
        assert_eq!(active_keyset.id, keyset_info.id);
        assert_eq!(active_keyset.active, keyset_info.active);
        assert_eq!(active_keyset.unit, keyset_info.unit);
        assert_eq!(active_keyset.input_fee_ppk, keyset_info.input_fee_ppk);
    }
}
