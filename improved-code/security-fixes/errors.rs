use anchor_lang::prelude::*;

#[error_code]
pub enum WagerError {
    #[msg("Game session is not in the correct state")]
    InvalidGameState,

    #[msg("Invalid team selection. Team must be 0 or 1")]
    InvalidTeamSelection,

    #[msg("Team is already full")]
    TeamIsFull,

    #[msg("Insufficient funds to join the game")]
    InsufficientFunds,

    #[msg("Invalid number of players for this game mode")]
    InvalidPlayerCount,

    #[msg("All players not joined")]
    NotAllPlayersJoined,

    #[msg("Game is not in completed state")]
    GameNotCompleted,

    #[msg("Only the game authority can distribute winnings")]
    UnauthorizedDistribution,

    #[msg("Invalid winning team selection")]
    InvalidWinningTeam,

    #[msg("Failed to calculate total pot due to arithmetic overflow")]
    TotalPotCalculationError,

    #[msg("No winners found in the winning team")]
    NoWinnersFound,

    #[msg("Failed to calculate per-player winnings")]
    WinningsCalculationError,

    #[msg("Failed to distribute all funds from game session")]
    IncompleteDistribution,

    #[msg("Invalid team")]
    InvalidTeam,

    #[msg("Player account not found in winners")]
    PlayerAccountNotFound,

    #[msg("Invalid winning team selection")]
    InvalidWinner,

    #[msg("Arithmetic error")]
    ArithmeticError,

    #[msg("Invalid mint address provided")]
    InvalidMint,

    #[msg("Invalid remaining accounts provided")]
    InvalidRemainingAccounts,

    #[msg("Invalid winner token account owner")]
    InvalidWinnerTokenAccount,

    #[msg("Invalid token mint")]
    InvalidTokenMint,

    #[msg("Invalid spawns")]
    InvalidSpawns,

    #[msg("Unauthorized kill")]
    UnauthorizedKill,

    #[msg("Unauthorized pay to spawn")]
    UnauthorizedPayToSpawn,

    #[msg("Player not found")]
    PlayerNotFound,

    #[msg("Invalid player token account")]
    InvalidPlayerTokenAccount,

    #[msg("Invalid player")]
    InvalidPlayer,

    #[msg("Player has no spawns")]
    PlayerHasNoSpawns,

    #[msg("Game is not in progress")]
    GameNotInProgress,

    // NEW ERROR TYPES - Added for security fixes
    #[msg("Player has already joined a team")]
    PlayerAlreadyJoined,

    #[msg("Bet amount must be greater than zero")]
    InvalidBetAmount,

    #[msg("Bet amount exceeds maximum allowed")]
    BetAmountTooHigh,

    #[msg("Insufficient balance in vault for distribution")]
    InsufficientVaultBalance,

    #[msg("Game session has expired")]
    GameSessionExpired,

    #[msg("Too many attempts, rate limit exceeded")]
    RateLimitExceeded,

    #[msg("Emergency pause is active")]
    EmergencyPauseActive,

    #[msg("Invalid session duration")]
    InvalidSessionDuration,

    #[msg("Unauthorized access attempt")]
    UnauthorizedAccess,

    #[msg("Invalid account ownership")]
    InvalidAccountOwnership,

    #[msg("Session is already finalized")]
    SessionAlreadyFinalized,

    #[msg("Insufficient vault authority")]
    InsufficientVaultAuthority,

    #[msg("Invalid game configuration")]
    InvalidGameConfiguration,

    #[msg("Player limit exceeded")]
    PlayerLimitExceeded,

    #[msg("Invalid spawn increment")]
    InvalidSpawnIncrement,

    #[msg("Concurrent modification detected")]
    ConcurrentModificationDetected,
}