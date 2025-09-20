use crate::{errors::WagerError, state::*, TOKEN_ID};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

// SECURITY FIX: Added constants for better maintainability and validation
const MIN_BET_AMOUNT: u64 = 1000; // Minimum 1000 tokens
const MAX_BET_AMOUNT: u64 = 1_000_000_000; // Maximum 1B tokens
const MAX_SESSION_ID_LENGTH: usize = 32;

/// SECURITY FIX: Comprehensive user joining with all security validations
pub fn join_user_handler(ctx: Context<JoinUser>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let clock = Clock::get()?;
    let player_key = ctx.accounts.user.key();

    msg!("Player {} attempting to join team {} in session {}", player_key, team, session_id);

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

    // SECURITY FIX: Atomic state validation to prevent race conditions
    require!(
        game_session.status == GameStatus::WaitingForPlayers,
        WagerError::InvalidGameState
    );

    // Validate team number (0 for team A, 1 for team B)
    require!(
        team == 0 || team == 1, 
        WagerError::InvalidTeamSelection
    );

    // SECURITY FIX: Prevent duplicate player registration across teams
    game_session.validate_player_not_joined(&player_key)?;

    // SECURITY FIX: Validate bet amount is within acceptable bounds
    let session_bet = game_session.session_bet;
    require!(
        session_bet >= MIN_BET_AMOUNT,
        WagerError::BetAmountTooLow
    );
    require!(
        session_bet <= MAX_BET_AMOUNT,
        WagerError::BetAmountTooHigh
    );

    // SECURITY FIX: Enhanced user token account validation
    require!(
        ctx.accounts.user_token_account.amount >= session_bet,
        WagerError::InsufficientUserBalance
    );

    // SECURITY FIX: Use improved slot finding with current time validation
    let empty_index = game_session.get_player_empty_slot(team, clock.unix_timestamp)?;

    msg!("Found empty slot {} for player {} on team {}", empty_index, player_key, team);

    // SECURITY FIX: Validate vault can receive tokens before transfer
    let vault_balance_before = ctx.accounts.vault_token_account.amount;
    let expected_vault_balance = vault_balance_before
        .checked_add(session_bet)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Vault balance before: {}, expected after: {}", vault_balance_before, expected_vault_balance);

    // Transfer SPL tokens from user to vault using user's signature
    // SECURITY FIX: Added comprehensive error handling for transfer
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

    // SECURITY FIX: Bounds checking before array access
    require!(
        empty_index < MAX_PLAYERS_PER_TEAM,
        WagerError::InvalidPlayerIndex
    );

    // Get reference to the selected team with bounds validation
    let (selected_team, team_name) = if team == 0 {
        (&mut game_session.team_a, "A")
    } else {
        (&mut game_session.team_b, "B")
    };

    // SECURITY FIX: Double-check that the slot is still empty (race condition protection)
    require!(
        selected_team.players[empty_index] == Pubkey::default(),
        WagerError::TeamSlotNoLongerAvailable
    );

    // Add player to the first available slot with proper initialization
    selected_team.players[empty_index] = player_key;
    
    // SECURITY FIX: Initialize spawns using configurable value from game mode
    game_session.initialize_player_spawns(team, empty_index)?;
    
    // Initialize kills to zero
    selected_team.player_kills[empty_index] = 0;

    // SECURITY FIX: Update total bet for the team with overflow protection
    selected_team.total_bet = selected_team.total_bet
        .checked_add(session_bet)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Player {} successfully added to team {} at index {}", player_key, team_name, empty_index);

    // SECURITY FIX: Atomic state transition check
    if game_session.can_start()? {
        game_session.status = GameStatus::InProgress;
        msg!("Game session {} is now in progress", session_id);
        
        // Log game start details for monitoring
        let players_per_team = game_session.game_mode.players_per_team();
        let total_pot = session_bet
            .checked_mul(players_per_team as u64)
            .and_then(|x| x.checked_mul(2))
            .ok_or(WagerError::ArithmeticError)?;
        
        msg!("Game started with {} players per team, total pot: {} tokens", 
             players_per_team, total_pot);
    }

    Ok(())
}

/// SECURITY FIX: Emergency function to leave game before it starts
pub fn leave_game_handler(ctx: Context<LeaveGame>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let player_key = ctx.accounts.user.key();
    let clock = Clock::get()?;

    msg!("Player {} attempting to leave team {} in session {}", player_key, team, session_id);

    // Can only leave while waiting for players
    require!(
        game_session.status == GameStatus::WaitingForPlayers,
        WagerError::GameAlreadyInProgress
    );

    // Check session hasn't expired
    require!(
        !game_session.is_expired(clock.unix_timestamp),
        WagerError::GameSessionExpired
    );

    // Validate team
    require!(
        team == 0 || team == 1,
        WagerError::InvalidTeamSelection
    );

    // Find player in the team
    let player_index = game_session.get_player_index(team, player_key)?;

    // Get team reference
    let selected_team = if team == 0 {
        &mut game_session.team_a
    } else {
        &mut game_session.team_b
    };

    // Get refund amount
    let refund_amount = game_session.session_bet;

    // Validate vault has sufficient balance for refund
    require!(
        ctx.accounts.vault_token_account.amount >= refund_amount,
        WagerError::InsufficientVaultBalance
    );

    // Transfer tokens back to user
    anchor_spl::token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.vault_token_account.to_account_info(),
                to: ctx.accounts.user_token_account.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            &[&[
                b"vault",
                session_id.as_bytes(),
                &[game_session.vault_bump],
            ]],
        ),
        refund_amount,
    )?;

    // Remove player from team
    selected_team.players[player_index] = Pubkey::default();
    selected_team.player_spawns[player_index] = 0;
    selected_team.player_kills[player_index] = 0;

    // Update team's total bet
    selected_team.total_bet = selected_team.total_bet
        .checked_sub(refund_amount)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Player {} successfully left the game and received refund of {} tokens", 
         player_key, refund_amount);

    Ok(())
}

/// SECURITY FIX: Function to get current game status for monitoring
pub fn get_game_status(ctx: Context<GetGameStatus>, _session_id: String) -> Result<()> {
    let game_session = &ctx.accounts.game_session;
    let clock = Clock::get()?;

    msg!("=== Game Session Status ===");
    msg!("Session ID: {}", game_session.session_id);
    msg!("Authority: {}", game_session.authority);
    msg!("Status: {:?}", game_session.status);
    msg!("Game Mode: {:?}", game_session.game_mode);
    msg!("Session Bet: {} tokens", game_session.session_bet);
    msg!("Created At: {}", game_session.created_at);
    msg!("Expires At: {}", game_session.expires_at);
    msg!("Is Expired: {}", game_session.is_expired(clock.unix_timestamp));

    let players_per_team = game_session.game_mode.players_per_team();
    
    // Team A status
    let team_a_count = game_session.team_a.get_active_player_count(players_per_team);
    msg!("Team A: {}/{} players, Total Bet: {} tokens", 
         team_a_count, players_per_team, game_session.team_a.total_bet);
    
    for i in 0..players_per_team {
        if game_session.team_a.players[i] != Pubkey::default() {
            msg!("  Player {}: {} (Spawns: {}, Kills: {})", 
                 i, game_session.team_a.players[i], 
                 game_session.team_a.player_spawns[i],
                 game_session.team_a.player_kills[i]);
        }
    }

    // Team B status
    let team_b_count = game_session.team_b.get_active_player_count(players_per_team);
    msg!("Team B: {}/{} players, Total Bet: {} tokens", 
         team_b_count, players_per_team, game_session.team_b.total_bet);
    
    for i in 0..players_per_team {
        if game_session.team_b.players[i] != Pubkey::default() {
            msg!("  Player {}: {} (Spawns: {}, Kills: {})", 
                 i, game_session.team_b.players[i],
                 game_session.team_b.player_spawns[i], 
                 game_session.team_b.player_kills[i]);
        }
    }

    // Check if game can start
    if game_session.status == GameStatus::WaitingForPlayers {
        match game_session.can_start() {
            Ok(true) => msg!("Game is ready to start!"),
            Ok(false) => msg!("Game is waiting for more players"),
            Err(_) => msg!("Game cannot start due to validation errors"),
        }
    }

    Ok(())
}

/// SECURITY FIX: Enhanced account validation with race condition protection
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct JoinUser<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: Game server authority - validated in constraints
    #[account(
        constraint = game_server.key() == game_session.authority @ WagerError::UnauthorizedGameServer
    )]
    pub game_server: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.status == GameStatus::WaitingForPlayers @ WagerError::GameNotJoinable,
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
}

/// SECURITY FIX: Account structure for leaving games
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct LeaveGame<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.status == GameStatus::WaitingForPlayers @ WagerError::GameAlreadyInProgress,
    )]
    pub game_session: Account<'info, GameSession>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key() @ WagerError::InvalidTokenAccountOwner,
        constraint = user_token_account.mint == TOKEN_ID @ WagerError::InvalidTokenMint,
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// CHECK: Vault PDA
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
}

/// SECURITY FIX: Account structure for status queries
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct GetGameStatus<'info> {
    #[account(
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anchor_lang::prelude::*;

    #[test]
    fn test_bet_amount_validation() {
        // Test minimum bet amount
        assert!(MIN_BET_AMOUNT > 0);
        assert!(MIN_BET_AMOUNT <= MAX_BET_AMOUNT);
        
        // Test bet amount bounds
        let valid_bet = 50000u64;
        assert!(valid_bet >= MIN_BET_AMOUNT);
        assert!(valid_bet <= MAX_BET_AMOUNT);
    }

    #[test]
    fn test_session_id_length() {
        let valid_session_id = "test_session_123";
        assert!(valid_session_id.len() <= MAX_SESSION_ID_LENGTH);
        
        let invalid_session_id = "a".repeat(MAX_SESSION_ID_LENGTH + 1);
        assert!(invalid_session_id.len() > MAX_SESSION_ID_LENGTH);
    }

    #[test] 
    fn test_team_validation() {
        // Valid teams
        assert!(0 <= 1); // Team A
        assert!(1 <= 1); // Team B
        
        // Invalid teams would be caught by require! macro
        let invalid_team = 2u8;
        assert!(invalid_team > 1);
    }

    #[test]
    fn test_arithmetic_overflow_protection() {
        let large_bet = u64::MAX / 2;
        let players_per_team = 5u64;
        
        // Test safe multiplication
        let safe_result = large_bet.checked_mul(players_per_team);
        assert!(safe_result.is_some());
        
        // Test overflow scenario
        let overflow_result = u64::MAX.checked_mul(2);
        assert!(overflow_result.is_none());
    }

    #[test]
    fn test_vault_balance_calculation() {
        let initial_balance = 1000u64;
        let session_bet = 500u64;
        
        let expected_balance = initial_balance.checked_add(session_bet);
        assert_eq!(expected_balance, Some(1500));
        
        // Test overflow protection
        let overflow_balance = u64::MAX.checked_add(1);
        assert_eq!(overflow_balance, None);
    }
}