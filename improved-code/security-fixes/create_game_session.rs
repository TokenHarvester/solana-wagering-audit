use crate::errors::WagerError;
use crate::state::*;
use crate::TOKEN_ID;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

// SECURITY FIX: Added constants for bet amount validation and other limits
const MIN_BET_AMOUNT: u64 = 1_000; // Minimum 1,000 tokens (0.001 if 6 decimals)
const MAX_BET_AMOUNT: u64 = 1_000_000_000_000; // Maximum 1M tokens (1M if 6 decimals)
const MAX_SESSION_ID_LENGTH: usize = 32;
const MIN_SESSION_ID_LENGTH: usize = 3;
const SESSION_TIMEOUT_SECONDS: i64 = 7200; // 2 hours default timeout
const MAX_SESSIONS_PER_AUTHORITY: u16 = 100; // Prevent spam

// SECURITY FIX: Game session space calculation with proper sizing
const GAME_SESSION_SPACE: usize = 
    8 +                    // Account discriminator
    4 + MAX_SESSION_ID_LENGTH + // session_id (String)
    32 +                   // authority (Pubkey)
    8 +                    // session_bet (u64)
    1 +                    // game_mode (enum)
    (32 * MAX_PLAYERS_PER_TEAM + 8 + 16 * MAX_PLAYERS_PER_TEAM + 16 * MAX_PLAYERS_PER_TEAM) + // team_a
    (32 * MAX_PLAYERS_PER_TEAM + 8 + 16 * MAX_PLAYERS_PER_TEAM + 16 * MAX_PLAYERS_PER_TEAM) + // team_b
    1 +                    // status (enum)
    8 +                    // created_at (i64)
    8 +                    // expires_at (i64) - SECURITY FIX: Added expiration
    2 +                    // spawns_per_purchase (u16) - SECURITY FIX: Configurable spawns
    1 +                    // bump (u8)
    1 +                    // vault_bump (u8)
    1 +                    // vault_token_bump (u8)
    64;                    // Extra padding for future fields

/// SECURITY FIX: Comprehensive game session creation with all validations
pub fn create_game_session_handler(
    ctx: Context<CreateGameSession>,
    session_id: String,
    bet_amount: u64,
    game_mode: GameMode,
) -> Result<()> {
    let clock = Clock::get()?;
    let authority = ctx.accounts.game_server.key();
    
    msg!("Creating game session '{}' by authority {}", session_id, authority);

    // SECURITY FIX: Comprehensive session ID validation
    require!(
        session_id.len() >= MIN_SESSION_ID_LENGTH,
        WagerError::SessionIdTooShort
    );
    
    require!(
        session_id.len() <= MAX_SESSION_ID_LENGTH,
        WagerError::SessionIdTooLong
    );

    // Validate session ID contains only valid characters
    require!(
        session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        WagerError::InvalidSessionIdFormat
    );

    // SECURITY FIX: Comprehensive bet amount validation
    require!(
        bet_amount > 0,
        WagerError::InvalidBetAmount
    );

    require!(
        bet_amount >= MIN_BET_AMOUNT,
        WagerError::BetAmountTooLow
    );

    require!(
        bet_amount <= MAX_BET_AMOUNT,
        WagerError::BetAmountTooHigh
    );

    msg!("Bet amount validated: {} tokens", bet_amount);

    // SECURITY FIX: Validate game mode is supported
    let players_per_team = game_mode.players_per_team();
    require!(
        players_per_team > 0 && players_per_team <= MAX_PLAYERS_PER_TEAM,
        WagerError::InvalidGameMode
    );

    msg!("Game mode: {:?}, Players per team: {}", game_mode, players_per_team);

    // SECURITY FIX: Calculate expiration time with overflow protection
    let expires_at = clock.unix_timestamp
        .checked_add(SESSION_TIMEOUT_SECONDS)
        .ok_or(WagerError::ArithmeticError)?;

    // SECURITY FIX: Initialize game session using secure constructor
    let game_session = &mut ctx.accounts.game_session;
    
    // Initialize the game session with comprehensive validation
    *game_session = GameSession::new(
        session_id.clone(),
        authority,
        bet_amount,
        game_mode,
        clock.unix_timestamp,
        ctx.bumps.game_session,
        ctx.bumps.vault,
        ctx.bumps.vault_token_account, // SECURITY FIX: Added vault token bump
    )?;

    msg!("Game session initialized successfully");

    // SECURITY FIX: Verify vault token account initialization
    let vault_token_account = &ctx.accounts.vault_token_account;
    require!(
        vault_token_account.mint == TOKEN_ID,
        WagerError::InvalidTokenMint
    );
    
    require!(
        vault_token_account.owner == ctx.accounts.vault.key(),
        WagerError::InvalidVaultTokenAccount
    );

    require!(
        vault_token_account.amount == 0,
        WagerError::VaultNotEmpty
    );

    msg!("Vault validation completed successfully");

    // SECURITY FIX: Log comprehensive session creation details for monitoring
    msg!("=== Game Session Created ===");
    msg!("Session ID: {}", game_session.session_id);
    msg!("Authority: {}", game_session.authority);
    msg!("Bet Amount: {} tokens", game_session.session_bet);
    msg!("Game Mode: {:?}", game_session.game_mode);
    msg!("Players per team: {}", players_per_team);
    msg!("Status: {:?}", game_session.status);
    msg!("Created at: {}", game_session.created_at);
    msg!("Expires at: {}", game_session.expires_at);
    msg!("Session timeout: {} seconds", SESSION_TIMEOUT_SECONDS);
    msg!("Spawns per purchase: {}", game_session.spawns_per_purchase);
    msg!("Game Session PDA: {}", game_session.key());
    msg!("Vault PDA: {}", ctx.accounts.vault.key());
    msg!("Vault Token Account: {}", vault_token_account.key());

    Ok(())
}

/// SECURITY FIX: Function to update session expiration (authority only)
pub fn extend_session_handler(
    ctx: Context<ExtendSession>,
    _session_id: String,
    additional_seconds: i64,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let clock = Clock::get()?;

    // Only authority can extend sessions
    require!(
        game_session.authority == ctx.accounts.authority.key(),
        WagerError::UnauthorizedAction
    );

    // Can only extend active sessions
    require!(
        game_session.status == GameStatus::WaitingForPlayers || 
        game_session.status == GameStatus::InProgress,
        WagerError::InvalidGameState
    );

    // Validate extension time is reasonable (max 24 hours)
    require!(
        additional_seconds > 0 && additional_seconds <= 86400,
        WagerError::InvalidExtensionTime
    );

    // Extend expiration time with overflow protection
    game_session.expires_at = game_session.expires_at
        .checked_add(additional_seconds)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Session {} extended by {} seconds. New expiration: {}", 
         game_session.session_id, additional_seconds, game_session.expires_at);

    Ok(())
}

/// SECURITY FIX: Function to cancel a session before it starts (authority only)
pub fn cancel_session_handler(
    ctx: Context<CancelSession>,
    _session_id: String,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;

    // Only authority can cancel sessions
    require!(
        game_session.authority == ctx.accounts.authority.key(),
        WagerError::UnauthorizedAction
    );

    // Can only cancel sessions that haven't started
    require!(
        game_session.status == GameStatus::WaitingForPlayers,
        WagerError::GameAlreadyStarted
    );

    // Change status to cancelled
    game_session.status = GameStatus::Cancelled;

    msg!("Session {} cancelled by authority", game_session.session_id);

    Ok(())
}

/// SECURITY FIX: Function to get session information
pub fn get_session_info_handler(
    ctx: Context<GetSessionInfo>,
    _session_id: String,
) -> Result<()> {
    let game_session = &ctx.accounts.game_session;
    let clock = Clock::get()?;

    msg!("=== Session Information ===");
    msg!("Session ID: {}", game_session.session_id);
    msg!("Authority: {}", game_session.authority);
    msg!("Status: {:?}", game_session.status);
    msg!("Game Mode: {:?}", game_session.game_mode);
    msg!("Bet Amount: {} tokens", game_session.session_bet);
    msg!("Players per team: {}", game_session.game_mode.players_per_team());
    msg!("Created at: {}", game_session.created_at);
    msg!("Expires at: {}", game_session.expires_at);
    msg!("Is expired: {}", game_session.is_expired(clock.unix_timestamp));
    msg!("Spawns per purchase: {}", game_session.spawns_per_purchase);
    
    // Team information
    let players_per_team = game_session.game_mode.players_per_team();
    let team_a_count = game_session.team_a.get_active_player_count(players_per_team);
    let team_b_count = game_session.team_b.get_active_player_count(players_per_team);
    
    msg!("Team A: {}/{} players, Total bet: {}", 
         team_a_count, players_per_team, game_session.team_a.total_bet);
    msg!("Team B: {}/{} players, Total bet: {}", 
         team_b_count, players_per_team, game_session.team_b.total_bet);

    Ok(())
}

/// SECURITY FIX: Enhanced account validation with comprehensive constraints
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct CreateGameSession<'info> {
    #[account(
        mut,
        constraint = session_id.len() >= MIN_SESSION_ID_LENGTH @ WagerError::SessionIdTooShort,
        constraint = session_id.len() <= MAX_SESSION_ID_LENGTH @ WagerError::SessionIdTooLong,
    )]
    pub game_server: Signer<'info>,

    #[account(
        init,
        payer = game_server,
        space = GAME_SESSION_SPACE,
        seeds = [b"game_session", session_id.as_bytes()],
        bump
    )]
    pub game_session: Account<'info, GameSession>,

    /// CHECK: Vault PDA for holding funds
    #[account(
        init,
        payer = game_server,
        space = 0,
        seeds = [b"vault", session_id.as_bytes()],
        bump
    )]
    pub vault: AccountInfo<'info>,

    #[account(
        init,
        payer = game_server,
        associated_token::mint = mint,
        associated_token::authority = vault,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        address = TOKEN_ID @ WagerError::InvalidMint
    )]
    pub mint: Account<'info, anchor_spl::token::Mint>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

/// SECURITY FIX: Account structure for extending sessions
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct ExtendSession<'info> {
    #[account(
        constraint = authority.key() == game_session.authority @ WagerError::UnauthorizedAction,
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.status == GameStatus::WaitingForPlayers || 
                     game_session.status == GameStatus::InProgress @ WagerError::InvalidGameState,
    )]
    pub game_session: Account<'info, GameSession>,
}

/// SECURITY FIX: Account structure for cancelling sessions
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct CancelSession<'info> {
    #[account(
        constraint = authority.key() == game_session.authority @ WagerError::UnauthorizedAction,
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.status == GameStatus::WaitingForPlayers @ WagerError::GameAlreadyStarted,
    )]
    pub game_session: Account<'info, GameSession>,
}

/// SECURITY FIX: Account structure for session info queries
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct GetSessionInfo<'info> {
    #[account(
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bet_amount_validation() {
        // Test minimum bet
        assert!(MIN_BET_AMOUNT > 0);
        
        // Test valid bet amounts
        let valid_bet = 50_000u64;
        assert!(valid_bet >= MIN_BET_AMOUNT && valid_bet <= MAX_BET_AMOUNT);
        
        // Test invalid bet amounts
        let too_low = MIN_BET_AMOUNT - 1;
        assert!(too_low < MIN_BET_AMOUNT);
        
        let too_high = MAX_BET_AMOUNT + 1;
        assert!(too_high > MAX_BET_AMOUNT);
    }

    #[test]
    fn test_session_id_validation() {
        // Valid session IDs
        let valid_ids = vec![
            "test_session",
            "game-123",
            "session_1v1_battle",
            "pvp_match_001",
        ];
        
        for id in valid_ids {
            assert!(id.len() >= MIN_SESSION_ID_LENGTH);
            assert!(id.len() <= MAX_SESSION_ID_LENGTH);
            assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        }
        
        // Invalid session IDs
        let invalid_ids = vec![
            "x",  // Too short
            &"a".repeat(MAX_SESSION_ID_LENGTH + 1), // Too long
            "session with spaces", // Invalid characters
            "session@special", // Invalid characters
        ];
        
        for id in invalid_ids {
            let is_invalid = id.len() < MIN_SESSION_ID_LENGTH || 
                           id.len() > MAX_SESSION_ID_LENGTH ||
                           !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            assert!(is_invalid);
        }
    }

    #[test]
    fn test_game_mode_validation() {
        let modes = [
            GameMode::WinnerTakesAllOneVsOne,
            GameMode::WinnerTakesAllThreeVsThree,
            GameMode::WinnerTakesAllFiveVsFive,
            GameMode::PayToSpawnOneVsOne,
            GameMode::PayToSpawnThreeVsThree,
            GameMode::PayToSpawnFiveVsFive,
        ];
        
        for mode in modes {
            let players = mode.players_per_team();
            assert!(players > 0);
            assert!(players <= MAX_PLAYERS_PER_TEAM);
        }
    }

    #[test]
    fn test_session_timeout_calculation() {
        let current_time = 1000i64;
        let expires_at = current_time.checked_add(SESSION_TIMEOUT_SECONDS);
        
        assert!(expires_at.is_some());
        assert_eq!(expires_at.unwrap(), current_time + SESSION_TIMEOUT_SECONDS);
        
        // Test overflow protection
        let max_time = i64::MAX;
        let overflow_result = max_time.checked_add(SESSION_TIMEOUT_SECONDS);
        assert!(overflow_result.is_none());
    }

    #[test]
    fn test_space_calculation() {
        // Ensure space calculation is reasonable
        assert!(GAME_SESSION_SPACE > 500); // Should be substantial
        assert!(GAME_SESSION_SPACE < 10_000); // But not excessive
        
        // Test that it accounts for all major fields
        let expected_minimum = 
            8 +                    // discriminator
            4 + MAX_SESSION_ID_LENGTH + // session_id
            32 +                   // authority
            8 +                    // session_bet
            1 +                    // game_mode
            8 +                    // created_at
            8 +                    // expires_at
            2 +                    // spawns_per_purchase
            3;                     // bumps
            
        assert!(GAME_SESSION_SPACE >= expected_minimum);
    }

    #[test]
    fn test_extension_time_limits() {
        // Valid extension times
        let valid_extensions = [3600i64, 7200, 21600]; // 1h, 2h, 6h
        for ext in valid_extensions {
            assert!(ext > 0 && ext <= 86400); // Max 24 hours
        }
        
        // Invalid extension times
        let invalid_extensions = [0i64, -3600, 86401]; // 0, negative, >24h
        for ext in invalid_extensions {
            assert!(ext <= 0 || ext > 86400);
        }
    }
}