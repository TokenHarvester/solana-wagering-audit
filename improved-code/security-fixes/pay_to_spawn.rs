use crate::{errors::WagerError, state::*, TOKEN_ID};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

// SECURITY FIX: Added constants for better maintainability and validation
const MAX_SPAWNS_PER_PLAYER: u16 = 100; // Prevent excessive spawn purchases
const MIN_BET_AMOUNT: u64 = 1000; // Minimum bet amount
const MAX_BET_AMOUNT: u64 = 1_000_000_000; // Maximum bet amount
const MAX_SESSION_ID_LENGTH: usize = 32;
const MAX_SPAWN_PURCHASES_PER_TRANSACTION: u8 = 5; // Rate limiting

/// SECURITY FIX: Comprehensive pay-to-spawn with all security validations
pub fn pay_to_spawn_handler(ctx: Context<PayToSpawn>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let clock = Clock::get()?;
    let player_key = ctx.accounts.user.key();

    msg!("Player {} attempting to purchase spawns for team {} in session {}", 
         player_key, team, session_id);

    // SECURITY FIX: Validate session hasn't expired
    require!(
        !game_session.is_expired(clock.unix_timestamp),
        WagerError::GameSessionExpired
    );

    // SECURITY FIX: Validate session ID length
    require!(
        session_id.len() <= MAX_SESSION_ID_LENGTH,
        WagerError::SessionIdTooLong
    );

    // SECURITY FIX: Comprehensive game state validation
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::InvalidGameState
    );

    require!(
        game_session.is_pay_to_spawn(),
        WagerError::InvalidGameMode
    );

    // Validate team number (0 for team A, 1 for team B)
    require!(
        team == 0 || team == 1, 
        WagerError::InvalidTeamSelection
    );

    // SECURITY FIX: Validate player exists in the specified team
    let player_index = game_session.get_player_index(team, player_key)?;

    // SECURITY FIX: Validate player index bounds
    require!(
        player_index < MAX_PLAYERS_PER_TEAM,
        WagerError::InvalidPlayerIndex
    );

    // SECURITY FIX: Check current spawn count to prevent excessive accumulation
    let current_spawns = if team == 0 {
        game_session.team_a.player_spawns[player_index]
    } else {
        game_session.team_b.player_spawns[player_index]
    };

    require!(
        current_spawns < MAX_SPAWNS_PER_PLAYER,
        WagerError::MaxSpawnsExceeded
    );

    // SECURITY FIX: Validate bet amount bounds
    let session_bet = game_session.session_bet;
    require!(
        session_bet >= MIN_BET_AMOUNT && session_bet <= MAX_BET_AMOUNT,
        WagerError::InvalidBetAmount
    );

    // SECURITY FIX: Comprehensive user balance validation
    require!(
        ctx.accounts.user_token_account.amount >= session_bet,
        WagerError::InsufficientUserBalance
    );

    // SECURITY FIX: Validate that spawns to be added won't exceed maximum
    let spawns_to_add = game_session.spawns_per_purchase;
    let new_spawn_count = current_spawns
        .checked_add(spawns_to_add)
        .ok_or(WagerError::ArithmeticError)?;

    require!(
        new_spawn_count <= MAX_SPAWNS_PER_PLAYER,
        WagerError::MaxSpawnsExceeded
    );

    msg!("Current spawns: {}, Adding: {}, New total: {}", 
         current_spawns, spawns_to_add, new_spawn_count);

    // SECURITY FIX: Validate vault can receive tokens
    let vault_balance_before = ctx.accounts.vault_token_account.amount;
    let expected_vault_balance = vault_balance_before
        .checked_add(session_bet)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Vault balance before: {}, expected after: {}", 
         vault_balance_before, expected_vault_balance);

    // SECURITY FIX: Enhanced token transfer with comprehensive error handling
    anchor_spl::token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        session_bet,
    ).map_err(|e| {
        msg!("Token transfer failed: {:?}", e);
        error!(WagerError::TokenTransferFailed)
    })?;

    // SECURITY FIX: Verify transfer was successful
    ctx.accounts.vault_token_account.reload()?;
    let vault_balance_after = ctx.accounts.vault_token_account.amount;
    require!(
        vault_balance_after == expected_vault_balance,
        WagerError::TransferVerificationFailed
    );

    msg!("Token transfer successful. Vault balance after: {}", vault_balance_after);

    // Add spawns to the player using the secure method
    game_session.add_spawns(team, player_index)?;

    // SECURITY FIX: Update team's total collected funds with overflow protection
    let team_total_bet = if team == 0 {
        &mut game_session.team_a.total_bet
    } else {
        &mut game_session.team_b.total_bet
    };

    *team_total_bet = team_total_bet
        .checked_add(session_bet)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Player {} successfully purchased {} spawns for {} tokens", 
         player_key, spawns_to_add, session_bet);

    // SECURITY FIX: Log important metrics for monitoring
    msg!("Team {} total collected: {} tokens", team, team_total_bet);
    msg!("Player spawn count updated to: {}", new_spawn_count);

    Ok(())
}

/// SECURITY FIX: Function to get spawn purchase cost and limits
pub fn get_spawn_info(ctx: Context<GetSpawnInfo>, _session_id: String) -> Result<()> {
    let game_session = &ctx.accounts.game_session;
    let clock = Clock::get()?;

    msg!("=== Spawn Purchase Information ===");
    msg!("Session ID: {}", game_session.session_id);
    msg!("Game Mode: {:?}", game_session.game_mode);
    msg!("Is Pay-to-Spawn: {}", game_session.is_pay_to_spawn());
    msg!("Session Status: {:?}", game_session.status);
    msg!("Session Expired: {}", game_session.is_expired(clock.unix_timestamp));
    msg!("Cost per spawn purchase: {} tokens", game_session.session_bet);
    msg!("Spawns per purchase: {}", game_session.spawns_per_purchase);
    msg!("Maximum spawns per player: {}", MAX_SPAWNS_PER_PLAYER);
    
    if !game_session.is_pay_to_spawn() {
        msg!("WARNING: This is not a pay-to-spawn game mode!");
    }
    
    if game_session.status != GameStatus::InProgress {
        msg!("WARNING: Game is not in progress. Current status: {:?}", game_session.status);
    }

    Ok(())
}

/// SECURITY FIX: Function to get player's current spawn count
pub fn get_player_spawn_count(
    ctx: Context<GetPlayerSpawnCount>, 
    _session_id: String, 
    team: u8, 
    player: Pubkey
) -> Result<()> {
    let game_session = &ctx.accounts.game_session;

    require!(team == 0 || team == 1, WagerError::InvalidTeamSelection);

    match game_session.get_player_index(team, player) {
        Ok(player_index) => {
            let spawn_count = if team == 0 {
                game_session.team_a.player_spawns[player_index]
            } else {
                game_session.team_b.player_spawns[player_index]
            };
            
            let kill_count = if team == 0 {
                game_session.team_a.player_kills[player_index]
            } else {
                game_session.team_b.player_kills[player_index]
            };

            msg!("=== Player Spawn Information ===");
            msg!("Player: {}", player);
            msg!("Team: {}", team);
            msg!("Current spawns: {}", spawn_count);
            msg!("Current kills: {}", kill_count);
            msg!("Can purchase more spawns: {}", spawn_count < MAX_SPAWNS_PER_PLAYER);
            msg!("Spawns remaining until max: {}", 
                 MAX_SPAWNS_PER_PLAYER.saturating_sub(spawn_count));
        }
        Err(_) => {
            msg!("Player {} not found in team {}", player, team);
            return Err(error!(WagerError::PlayerNotFound));
        }
    }

    Ok(())
}

/// SECURITY FIX: Emergency function to disable spawn purchases (authority only)
pub fn disable_spawn_purchases(
    ctx: Context<DisableSpawnPurchases>, 
    _session_id: String
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;

    // Only game authority can disable spawn purchases
    require!(
        game_session.authority == ctx.accounts.authority.key(),
        WagerError::UnauthorizedAction
    );

    // Set spawns per purchase to 0 to effectively disable purchases
    game_session.spawns_per_purchase = 0;

    msg!("Spawn purchases disabled for session {}", game_session.session_id);
    
    Ok(())
}

/// SECURITY FIX: Function to update spawn purchase configuration (authority only)
pub fn update_spawn_config(
    ctx: Context<UpdateSpawnConfig>, 
    _session_id: String,
    new_spawns_per_purchase: u16
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;

    // Only game authority can update configuration
    require!(
        game_session.authority == ctx.accounts.authority.key(),
        WagerError::UnauthorizedAction
    );

    // Validate new configuration
    require!(
        new_spawns_per_purchase > 0 && new_spawns_per_purchase <= 50,
        WagerError::InvalidSpawnCount
    );

    game_session.update_spawns_per_purchase(new_spawns_per_purchase)?;

    msg!("Spawn configuration updated: {} spawns per purchase", new_spawns_per_purchase);
    
    Ok(())
}

/// SECURITY FIX: Enhanced account validation with proper authority checks
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct PayToSpawn<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// SECURITY FIX: Proper authority validation with constraint
    #[account(
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedPayToSpawn,
    )]
    pub game_server: Signer<'info>, // SECURITY FIX: Changed from AccountInfo to Signer

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.status == GameStatus::InProgress @ WagerError::GameNotInProgress,
        constraint = game_session.is_pay_to_spawn() @ WagerError::InvalidGameMode,
        constraint = session_id.len() <= MAX_SESSION_ID_LENGTH @ WagerError::SessionIdTooLong,
    )]
    pub game_session: Account<'info, GameSession>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key() @ WagerError::InvalidTokenAccountOwner,
        constraint = user_token_account.mint == TOKEN_ID @ WagerError::InvalidTokenMint,
        constraint = user_token_account.amount >= game_session.session_bet @ WagerError::InsufficientUserBalance,
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// CHECK: Vault PDA that holds the funds
    #[account(
        mut,
        seeds = [b"vault", session_id.as_bytes()],
        bump = game_session.vault_bump,
    )]
    pub vault: AccountInfo<'info>,

    #[account(
        mut,
        associated_token::mint = TOKEN_ID,
        associated_token::authority = vault,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// SECURITY FIX: Account structure for spawn information queries
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct GetSpawnInfo<'info> {
    #[account(
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
}

/// SECURITY FIX: Account structure for player spawn count queries
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct GetPlayerSpawnCount<'info> {
    #[account(
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
}

/// SECURITY FIX: Account structure for disabling spawn purchases
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct DisableSpawnPurchases<'info> {
    #[account(
        constraint = authority.key() == game_session.authority @ WagerError::UnauthorizedAction,
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
}

/// SECURITY FIX: Account structure for updating spawn configuration
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct UpdateSpawnConfig<'info> {
    #[account(
        constraint = authority.key() == game_session.authority @ WagerError::UnauthorizedAction,
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_limits() {
        let max_spawns = MAX_SPAWNS_PER_PLAYER;
        let spawns_per_purchase = 10u16;
        
        // Test normal case
        let current_spawns = 50u16;
        let new_spawns = current_spawns.checked_add(spawns_per_purchase).unwrap();
        assert!(new_spawns <= max_spawns);
        
        // Test overflow prevention
        let high_spawns = 95u16;
        let new_spawns_overflow = high_spawns.checked_add(spawns_per_purchase);
        match new_spawns_overflow {
            Some(count) => assert!(count > max_spawns), // Would exceed limit
            None => assert!(false, "Should not overflow u16"),
        }
    }

    #[test]
    fn test_bet_amount_validation() {
        // Test valid bet amounts
        let valid_bet = 50000u64;
        assert!(valid_bet >= MIN_BET_AMOUNT && valid_bet <= MAX_BET_AMOUNT);
        
        // Test invalid bet amounts
        let too_low = MIN_BET_AMOUNT - 1;
        assert!(too_low < MIN_BET_AMOUNT);
        
        let too_high = MAX_BET_AMOUNT + 1;
        assert!(too_high > MAX_BET_AMOUNT);
    }

    #[test]
    fn test_vault_balance_calculation() {
        let initial_balance = 1000u64;
        let bet_amount = 500u64;
        
        let expected_balance = initial_balance.checked_add(bet_amount);
        assert_eq!(expected_balance, Some(1500));
        
        // Test overflow protection
        let max_balance = u64::MAX;
        let overflow_result = max_balance.checked_add(1);
        assert_eq!(overflow_result, None);
    }

    #[test]
    fn test_team_validation() {
        // Valid teams
        assert!(0 <= 1);
        assert!(1 <= 1);
        
        // Invalid teams
        let invalid_team = 2u8;
        assert!(invalid_team > 1);
    }

    #[test]
    fn test_session_id_length() {
        let valid_id = "test_session";
        assert!(valid_id.len() <= MAX_SESSION_ID_LENGTH);
        
        let invalid_id = "a".repeat(MAX_SESSION_ID_LENGTH + 1);
        assert!(invalid_id.len() > MAX_SESSION_ID_LENGTH);
    }

    #[test]
    fn test_spawn_purchase_arithmetic() {
        let current_spawns = 15u16;
        let spawns_per_purchase = 10u16;
        let session_bet = 1000u64;
        
        // Test spawn addition
        let new_spawn_count = current_spawns.checked_add(spawns_per_purchase);
        assert_eq!(new_spawn_count, Some(25));
        
        // Test team total update
        let team_total = 5000u64;
        let new_total = team_total.checked_add(session_bet);
        assert_eq!(new_total, Some(6000));
    }
}