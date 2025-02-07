pub use anchor_lang::{
    solana_program::{
        sysvar::rent::ID as RENT_ID,
        program::{invoke, invoke_signed}
    },
    prelude::*
};
pub use anchor_spl::token_2022::Token2022;
pub use spl_token_2022::{
    extension::ExtensionType,
    instruction::{initialize_mint_close_authority, initialize_permanent_delegate, initialize_mint2},
    extension::metadata_pointer::instruction::initialize as initialize_metadata_pointer,
};
pub use spl_token_metadata_interface::{
    state::{TokenMetadata, Field},
    instruction::{initialize as initialize_metadata_account, update_field as update_metadata_account},
};
pub use crate::state::{Protocol, Collection, Admin, Placeholder};
pub use crate::errors::{BuyingError, ProtocolError};

#[derive(Accounts)]
#[instruction(id: u64)]
pub struct CreatePlaceholder<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        seeds = [b"admin_state", admin.key().as_ref()],
        bump
    )]
    pub admin_state: Account<'info, Admin>,
    #[account(
        seeds = [b"collection", collection.owner.key().as_ref()],
        bump,
    )] 
    pub collection: Account<'info, Collection>,
    #[account(
        init,
        payer = admin,
        seeds = [b"placeholder", collection.key().as_ref(), id.to_le_bytes().as_ref()],
        bump,
        space = Placeholder::INIT_SPACE + 32 + collection.name.len() + collection.symbol.len() + 8 + 8,
    )] 
    pub placeholder: Account<'info, Placeholder>,
    /// CHECK: this is fine since we are handling all the checks and creation in the program.
    #[account(
        mut,
        seeds = [b"mint", placeholder.key().as_ref()],
        bump
    )]
    pub mint: UncheckedAccount<'info>,
    /// CHECK:
    #[account(
        seeds = [b"auth"],
        bump
    )]
    pub auth: UncheckedAccount<'info>,
    #[account(address = RENT_ID)]
    /// CHECK: this is fine since we are hard coding the rent sysvar.
    pub rent: UncheckedAccount<'info>,
    pub token_2022_program: Program<'info, Token2022>,
    #[account(
        seeds = [b"protocol"],
        bump,
    )]
    pub protocol: Account<'info, Protocol>,
    pub system_program: Program<'info, System>,
}

impl<'info> CreatePlaceholder<'info> {
    pub fn create(
        &mut self,
        id: u64,
        uri: String,
        bumps: CreatePlaceholderBumps,
    ) -> Result<()> {
        /*
        
            Create Placeholder Nft Ix:

            Some security check:
            - The admin_state.publickey must match the signing admin.

            What these Instructions do:
            - Creates a placeholder NFT.
        */

        require!(!self.protocol.locked, ProtocolError::ProtocolLocked);
        require!(self.admin_state.publickey == *self.admin.key, ProtocolError::UnauthorizedAdmin);
        
        if self.collection.total_supply > self.collection.max_supply{
            return Err(BuyingError::SoldOut.into());
        }

        self.placeholder.set_inner(
            Placeholder {
                id, // 8
                collection: self.collection.key(),  //32 bytes
                reference: self.collection.reference.to_string(), // 4 + len
                name: self.collection.name.clone(), // 4 + len
                price: self.collection.price, // 8
                time_stamp: Clock::get()?.unix_timestamp // 8
            }
        );

        // Step 1: Initialize Account
        let size = ExtensionType::try_calculate_account_len::<spl_token_2022::state::Mint>(
            &[
                ExtensionType::MintCloseAuthority,
                ExtensionType::PermanentDelegate,
                ExtensionType::MetadataPointer,
            ],
        ).unwrap();
        let _count = self.collection.total_supply + 1;
        let metadata = TokenMetadata {
            update_authority: spl_pod::optional_keys::OptionalNonZeroPubkey::try_from(Some(self.auth.key())).unwrap(),
            mint: self.mint.key(),
            name: "Placeholder for".to_string() + &self.collection.name,
            symbol: self.collection.symbol.clone(),
            uri,
            additional_metadata: vec![
                ("id".to_string(), id.to_string()),
                ("count".to_string(), _count.to_string()),
                ("timestamp".to_string(), Clock::get()?.unix_timestamp.to_string()),
                ("price".to_string(), self.collection.price.to_string()),
                ("collection".to_string(), self.collection.name.to_string()),
                ("collection key".to_string(), self.collection.key().to_string())
            ]
        };

        let extension_extra_space = metadata.tlv_size_of().unwrap();
        let rent = &Rent::from_account_info(&self.rent.to_account_info())?;
        let lamports = rent.minimum_balance(size + extension_extra_space);

        let placeholder_key = self.placeholder.key();
        let seeds: &[&[u8]; 3] = &[
            b"mint",
            placeholder_key.as_ref(),
            &[bumps.mint],
        ];
        let signer_seeds = &[&seeds[..]];

        invoke_signed(
            &solana_program::system_instruction::create_account(
                &self.admin.key(),
                &self.mint.key(),
                lamports,
                (size).try_into().unwrap(),
                &spl_token_2022::id(),
            ),
            &vec![
                self.admin.to_account_info(),
                self.mint.to_account_info(),
            ],
            signer_seeds
        )?;

        // Step 2: Initialize Extension needed: 

        // 2.1: Permanent Delegate, 
        invoke(
            &initialize_permanent_delegate(
                &self.token_2022_program.key(),
                &self.mint.key(),
                &self.auth.key(),
            )?,
            &vec![
                self.mint.to_account_info(),
            ],
        )?;
        
        // 2.2: Close Mint Authority, 
        invoke(
            &initialize_mint_close_authority(
                &self.token_2022_program.key(),
                &self.mint.key(),
                Some(&self.auth.key()),
            )?,
            &vec![
                self.mint.to_account_info(),
            ],
        )?;
        
        // 2.3: Metadata Pointer
        invoke(
            &initialize_metadata_pointer(
                &self.token_2022_program.key(),
                &self.mint.key(),
                Some(self.auth.key()),
                Some(self.mint.key()),
            )?,
            &vec![
                self.mint.to_account_info(),
            ],
        )?;

        // Step 3: Initialize Mint & Metadata Account
        invoke_signed(
            &initialize_mint2(
                &self.token_2022_program.key(),
                &self.mint.key(),
                &self.auth.key(),
                None,
                0,
            )?,
            &vec![
                self.mint.to_account_info(),
            ],
            signer_seeds
        )?;

        let seeds: &[&[u8]; 2] = &[
            b"auth",
            &[bumps.auth],
        ];
        let signer_seeds = &[&seeds[..]];

        invoke_signed(
            &initialize_metadata_account(
                &self.token_2022_program.key(),
                &self.mint.key(),
                &self.auth.key(),
                &self.mint.key(),
                &self.auth.key(),
                metadata.name,
                metadata.symbol,
                metadata.uri,
            ),
            &vec![
                self.mint.to_account_info(),
                self.auth.to_account_info(),
            ],
            signer_seeds
        )?;

        for (field, value) in metadata.additional_metadata.into_iter() {
            invoke_signed(
                &update_metadata_account(
                    &self.token_2022_program.key(),
                    &self.mint.key(),
                    &self.auth.key(),
                    Field::Key(field),
                    value,
                ),
                &vec![
                    self.mint.to_account_info(),
                    self.auth.to_account_info(),
                ],
                signer_seeds
            )?;
        }

       Ok(())
    }
}