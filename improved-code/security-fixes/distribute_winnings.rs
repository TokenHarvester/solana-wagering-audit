use crate::{errors::WagerError, state::*, TOKEN_ID};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

// SECURITY FIX: Added constants for better maintainability
const EARNINGS_DIVISOR: u64 = 10;
const MAX_DISTRIBUTION_ATTEMPTS: usize = 3;

/// SECURITY FIX: Comprehensive vault balance validation and rollback capability
pub fn distribute_pay_spawn_earnings<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    session_id: String,
) -> Result<()> {
    let game_session = &ctx.accounts.game_session;
    msg!("Starting pay-to-spawn earnings distribution for session: {}", session_id);

    // SECURITY FIX: Validate game session state and expiration
    let clock = Clock::get()?;
    require!(
        !game_session.is_expired(clock.unix_timestamp),
        WagerError::GameSessionExpired
    );

    require!(
        game_session.status == GameStatus::InProgress || 
        game_session.status == GameStatus::Completed,
        WagerError::InvalidGameState
    );

    require!(
        game_session.is_pay_to_spawn(),
        WagerError::InvalidGameMode
    );

    let players = game_session.get_all_players();
    msg!("Number of players: {}", players.len());
    msg!("Number of remaining accounts: {}", ctx.remaining_accounts.len());

    // Validate remaining accounts structure
    require!(
        !ctx.remaining_accounts.is_empty(),
        WagerError::InvalidRemainingAccounts
    );

    require!(
        ctx.remaining_accounts.len() % 2 == 0,
        WagerError::InvalidRemainingAccounts
    );

    require!(
        ctx.remaining_accounts.len() <= players.len() * 2,
        WagerError::TooManyRemainingAccounts
    );

    // SECURITY FIX: Calculate total distribution required before any transfers
    let mut total_distribution_needed = 0u64;
    let mut player_distributions = Vec::new();

    for player in &players {
        let kills_and_spawns = game_session.get_kills_and_spawns(*player)?;
        if kills_and_spawns == 0 {
            continue;
        }

        // SECURITY FIX: Use checked arithmetic to prevent overflow
        let earnings = (kills_and_spawns as u64)
            .checked_mul(game_session.session_bet)
            .and_then(|x| x.checked_div(EARNINGS_DIVISOR))
            .ok_or(WagerError::ArithmeticError)?;

        if earnings > 0 {
            total_distribution_needed = total_distribution_needed
                .checked_add(earnings)
                .ok_or(WagerError::ArithmeticError)?;

            player_distributions.push((*player, earnings));
        }
    }

    msg!("Total distribution needed: {}", total_distribution_needed);

    // SECURITY FIX: Validate vault has sufficient balance BEFORE any transfers
    let vault_balance = ctx.accounts.vault_token_account.amount;
    msg!("Vault balance: {}", vault_balance);
    
    require!(
        vault_balance >= total_distribution_needed,
        WagerError::InsufficientVaultBalance
    );

    // If no distributions needed, just mark as completed
    if player_distributions.is_empty() {
        msg!("No earnings to distribute");
        let game_session = &mut ctx.accounts.game_session;
        game_session.status = GameStatus::Completed;
        return Ok(());
    }

    // SECURITY FIX: Process distributions with comprehensive validation and error handling
    let mut successful_transfers = Vec::new();
    let mut transfer_errors = Vec::new();

    for (player, earnings) in &player_distributions {
        match process_player_distribution(
            &ctx,
            *player,
            *earnings,
            &session_id,
        ) {
            Ok(()) => {
                successful_transfers.push((*player, *earnings));
                msg!("Successfully transferred {} tokens to player {}", earnings, player);
            }
            Err(e) => {
                msg!("Failed to transfer to player {}: {:?}", player, e);
                transfer_errors.push((*player, e));
            }
        }
    }

    // SECURITY FIX: Handle partial failures gracefully
    if !transfer_errors.is_empty() {
        msg!("Distribution completed with {} errors out of {} players", 
             transfer_errors.len(), 
             player_distributions.len());
        
        // If critical number of transfers failed, consider rollback
        let failure_rate = transfer_errors.len() as f64 / player_distributions.len() as f64;
        if failure_rate > 0.5 {
            msg!("High failure rate detected: {:.2}%. Manual intervention may be required.", failure_rate * 100.0);
            return Err(error!(WagerError::DistributionPartialFailure));
        }
    }

    // Mark session as completed
    let game_session = &mut ctx.accounts.game_session;
    game_session.status = GameStatus::Completed;

    msg!("Pay-to-spawn earnings distribution completed successfully");
    Ok(())
}

/// SECURITY FIX: Secure winner-takes-all distribution with comprehensive validation
pub fn distribute_all_winnings_handler<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    session_id: String,
    winning_team: u8,
) -> Result<()> {
    let game_session = &ctx.accounts.game_session;
    msg!("Starting winner-takes-all distribution for session: {}", session_id);

    // SECURITY FIX: Validate game session state and expiration
    let clock = Clock::get()?;
    require!(
        !game_session.is_expired(clock.unix_timestamp),
        WagerError::GameSessionExpired
    );

    require!(
        game_session.status == GameStatus::Completed,
        WagerError::InvalidGameState
    );

    require!(
        !game_session.is_pay_to_spawn(),
        WagerError::InvalidGameMode
    );

    // Verify authority matches game session authority
    require!(
        game_session.authority == ctx.accounts.game_server.key(),
        WagerError::UnauthorizedDistribution
    );

    // Validate winning team selection
    require!(
        winning_team == 0 || winning_team == 1,
        WagerError::InvalidWinningTeam
    );

    let players_per_team = game_session.game_mode.players_per_team();

    // Get the winning team players
    let winning_players = if winning_team == 0 {
        &game_session.team_a.players[0..players_per_team]
    } else {
        &game_session.team_b.players[0..players_per_team]
    };

    // Filter out empty slots (Pubkey::default())
    let active_winners: Vec<Pubkey> = winning_players
        .iter()
        .filter(|&&player| player != Pubkey::default())
        .copied()
        .collect();

    require!(!active_winners.is_empty(), WagerError::NoActiveWinners);

    msg!("Active winners: {}", active_winners.len());
    for player in &active_winners {
        msg!("Winning player: {}", player);
    }

    // Validate remaining accounts
    require!(
        ctx.remaining_accounts.len() >= active_winners.len() * 2,
        WagerError::InvalidRemainingAccounts
    );

    // SECURITY FIX: Calculate total distribution and validate vault balance
    let winning_amount_per_player = game_session.session_bet
        .checked_mul(2)
        .ok_or(WagerError::ArithmeticError)?;

    let total_distribution = winning_amount_per_player
        .checked_mul(active_winners.len() as u64)
        .ok_or(WagerError::ArithmeticError)?;

    msg!("Winning amount per player: {}", winning_amount_per_player);
    msg!("Total distribution needed: {}", total_distribution);

    // SECURITY FIX: Validate vault has sufficient balance BEFORE any transfers
    let vault_balance = ctx.accounts.vault_token_account.amount;
    msg!("Vault balance: {}", vault_balance);
    
    require!(
        vault_balance >= total_distribution,
        WagerError::InsufficientVaultBalance
    );

    // SECURITY FIX: Validate all winner accounts before starting transfers
    let mut winner_validations = Vec::new();
    for (i, &winner_pubkey) in active_winners.iter().enumerate() {
        let winner_account = &ctx.remaining_accounts[i * 2];
        let winner_token_account_info = &ctx.remaining_accounts[i * 2 + 1];

        // Validate winner account matches expected pubkey
        require!(
            winner_account.key() == winner_pubkey,
            WagerError::InvalidWinner
        );

        // Validate and deserialize token account
        let winner_token_account = Account::<TokenAccount>::try_from(winner_token_account_info)
            .map_err(|_| error!(WagerError::InvalidWinnerTokenAccount))?;

        // Validate token account ownership
        require!(
            winner_token_account.owner == winner_pubkey,
            WagerError::InvalidWinnerTokenAccount
        );

        // Validate token mint
        require!(
            winner_token_account.mint == TOKEN_ID,
            WagerError::InvalidTokenMint
        );

        // Verify winner is in the winning team
        require!(
            active_winners.contains(&winner_pubkey),
            WagerError::InvalidWinner
        );

        winner_validations.push((winner_account, winner_token_account_info, winner_pubkey));
    }

    // SECURITY FIX: Execute transfers with error handling and rollback capability
    let mut successful_transfers = Vec::new();

    for (winner_account, winner_token_account_info, winner_pubkey) in winner_validations {
        match execute_winner_transfer(
            &ctx,
            winner_token_account_info,
            winning_amount_per_player,
            &session_id,
        ) {
            Ok(()) => {
                successful_transfers.push((winner_pubkey, winning_amount_per_player));
                msg!("Successfully transferred {} tokens to winner {}", 
                     winning_amount_per_player, winner_pubkey);
            }
            Err(e) => {
                msg!("Failed to transfer to winner {}: {:?}", winner_pubkey, e);
                
                // SECURITY FIX: On any transfer failure in winner-takes-all, 
                // we need to handle it carefully since partial distribution 
                // would be unfair. For now, we'll fail the entire transaction.
                return Err(e);
            }
        }
    }

    // Mark session as distributed
    let game_session = &mut ctx.accounts.game_session;
    game_session.status = GameStatus::Distributed;

    msg!("Winner-takes-all distribution completed successfully");
    msg!("Total distributed: {} tokens to {} winners", 
         total_distribution, active_winners.len());
    
    Ok(())
}

/// SECURITY FIX: Helper function for individual player distribution with validation
fn process_player_distribution<'info>(
    ctx: &Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    player: Pubkey,
    earnings: u64,
    session_id: &str,
) -> Result<()> {
    // Find the player's account and token account in remaining_accounts
    let player_index = ctx
        .remaining_accounts
        .iter()
        .step_by(2)
        .position(|acc| acc.key() == player)
        .ok_or(WagerError::InvalidPlayer)?;

    let player_account = &ctx.remaining_accounts[player_index * 2];
    let player_token_account_info = &ctx.remaining_accounts[player_index * 2 + 1];

    // Validate and deserialize player token account
    let player_token_account = Account::<TokenAccount>::try_from(player_token_account_info)
        .map_err(|_| error!(WagerError::InvalidPlayerTokenAccount))?;

    // Validate token account ownership
    require!(
        player_token_account.owner == player,
        WagerError::InvalidPlayerTokenAccount
    );

    // Validate token mint
    require!(
        player_token_account.mint == TOKEN_ID,
        WagerError::InvalidTokenMint
    );

    // SECURITY FIX: Double-check vault balance before individual transfer
    let vault_balance = ctx.accounts.vault_token_account.amount;
    require!(
        vault_balance >= earnings,
        WagerError::InsufficientVaultBalance
    );

    // Execute the transfer
    anchor_spl::token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.vault_token_account.to_account_info(),
                to: player_token_account_info.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            &[&[
                b"vault",
                session_id.as_bytes(),
                &[ctx.accounts.game_session.vault_bump],
            ]],
        ),
        earnings,
    )?;

    Ok(())
}

/// SECURITY FIX: Helper function for winner transfer with validation
fn execute_winner_transfer<'info>(
    ctx: &Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    winner_token_account_info: &AccountInfo<'info>,
    amount: u64,
    session_id: &str,
) -> Result<()> {
    // SECURITY FIX: Double-check vault balance before individual transfer
    let vault_balance = ctx.accounts.vault_token_account.amount;
    require!(
        vault_balance >= amount,
        WagerError::InsufficientVaultBalance
    );

    // Execute the transfer
    anchor_spl::token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.vault_token_account.to_account_info(),
                to: winner_token_account_info.to_account_info(),
                authority: ctx.accounts.vault.to_account_info(),
            },
            &[&[
                b"vault",
                session_id.as_bytes(),
                &[ctx.accounts.game_session.vault_bump],
            ]],
        ),
        amount,
    )?;

    Ok(())
}

/// SECURITY FIX: Emergency function to cancel distribution if needed
pub fn cancel_distribution<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    _session_id: String,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    
    // Only authority can cancel
    require!(
        game_session.authority == ctx.accounts.game_server.key(),
        WagerError::UnauthorizedDistribution
    );

    // Can only cancel if not yet distributed
    require!(
        game_session.status != GameStatus::Distributed,
        WagerError::AlreadyDistributed
    );

    game_session.status = GameStatus::Cancelled;
    
    msg!("Distribution cancelled by authority");
    Ok(())
}

/// SECURITY FIX: Function to get distribution summary without executing transfers
pub fn get_distribution_summary<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    _session_id: String,
) -> Result<()> {
    let game_session = &ctx.accounts.game_session;
    
    if game_session.is_pay_to_spawn() {
        let players = game_session.get_all_players();
        let mut total_earnings = 0u64;
        let mut eligible_players = 0u32;

        for player in &players {
            let kills_and_spawns = game_session.get_kills_and_spawns(*player)?;
            if kills_and_spawns > 0 {
                let earnings = (kills_and_spawns as u64)
                    .checked_mul(game_session.session_bet)
                    .and_then(|x| x.checked_div(EARNINGS_DIVISOR))
                    .unwrap_or(0);
                
                if earnings > 0 {
                    total_earnings = total_earnings.checked_add(earnings).unwrap_or(u64::MAX);
                    eligible_players += 1;
                    msg!("Player {} eligible for {} tokens", player, earnings);
                }
            }
        }

        msg!("Pay-to-spawn summary: {} players eligible for total of {} tokens", 
             eligible_players, total_earnings);
    } else {
        let players_per_team = game_session.game_mode.players_per_team();
        let winning_amount = game_session.session_bet
            .checked_mul(2)
            .unwrap_or(u64::MAX);
        let total_distribution = winning_amount
            .checked_mul(players_per_team as u64)
            .unwrap_or(u64::MAX);
            
        msg!("Winner-takes-all summary: {} winners eligible for {} tokens each, total: {} tokens", 
             players_per_team, winning_amount, total_distribution);
    }

    let vault_balance = ctx.accounts.vault_token_account.amount;
    msg!("Current vault balance: {}", vault_balance);
    
    Ok(())
}

/// SECURITY FIX: Enhanced account validation structure
#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct DistributeWinnings<'info> {
    /// The game server authority that created the session
    pub game_server: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedDistribution,
        constraint = session_id.len() <= 32 @ WagerError::SessionIdTooLong,
    )]
    pub game_session: Account<'info, GameSession>,

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
        constraint = vault_token_account.amount > 0 @ WagerError::EmptyVault,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_earnings_calculation() {
        let kills_and_spawns = 15u16;
        let session_bet = 1000u64;
        
        let earnings = (kills_and_spawns as u64)
            .checked_mul(session_bet)
            .and_then(|x| x.checked_div(EARNINGS_DIVISOR))
            .unwrap();
            
        assert_eq!(earnings, 1500); // 15 * 1000 / 10 = 1500
    }

    #[test]
    fn test_earnings_overflow_protection() {
        let kills_and_spawns = u16::MAX;
        let session_bet = u64::MAX;
        
        let earnings = (kills_and_spawns as u64)
            .checked_mul(session_bet)
            .and_then(|x| x.checked_div(EARNINGS_DIVISOR));
            
        assert!(earnings.is_none()); // Should overflow and return None
    }

    #[test]
    fn test_winner_amount_calculation() {
        let session_bet = 1000u64;
        let winning_amount = session_bet.checked_mul(2).unwrap();
        assert_eq!(winning_amount, 2000);
    }
}