//! State accounts for the betting program
use crate::errors::WagerError;
use anchor_lang::prelude::*;

// Constants to replace magic numbers
pub const DEFAULT_SPAWN_COUNT: u16 = 10;
pub const MAX_PLAYERS_PER_TEAM: usize = 5;
pub const MAX_SESSION_ID_LENGTH: usize = 32;
pub const SESSION_TIMEOUT_SECONDS: i64 = 7200; // 2 hours

/// Game mode defining the team sizes
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq)]
pub enum GameMode {
    WinnerTakesAllOneVsOne,     // 1v1 game mode
    WinnerTakesAllThreeVsThree, // 3v3 game mode
    WinnerTakesAllFiveVsFive,   // 5v5 game mode
    PayToSpawnOneVsOne,         // 1v1 game mode
    PayToSpawnThreeVsThree,     // 3v3 game mode
    PayToSpawnFiveVsFive,       // 5v5 game mode
}

impl GameMode {
    /// Returns the required number of players per team
    pub fn players_per_team(&self) -> usize {
        match self {
            Self::WinnerTakesAllOneVsOne => 1,
            Self::WinnerTakesAllThreeVsThree => 3,
            Self::WinnerTakesAllFiveVsFive => 5,
            Self::PayToSpawnOneVsOne => 1,
            Self::PayToSpawnThreeVsThree => 3,
            Self::PayToSpawnFiveVsFive => 5,
        }
    }

    /// Returns whether this is a pay-to-spawn game mode
    pub fn is_pay_to_spawn(&self) -> bool {
        matches!(
            self,
            Self::PayToSpawnOneVsOne
                | Self::PayToSpawnThreeVsThree
                | Self::PayToSpawnFiveVsFive
        )
    }

    /// Returns the default spawn count for this game mode
    pub fn default_spawn_count(&self) -> u16 {
        match self {
            Self::WinnerTakesAllOneVsOne
            | Self::WinnerTakesAllThreeVsThree
            | Self::WinnerTakesAllFiveVsFive => 1, // Winner takes all: single life
            Self::PayToSpawnOneVsOne
            | Self::PayToSpawnThreeVsThree
            | Self::PayToSpawnFiveVsFive => DEFAULT_SPAWN_COUNT, // Pay-to-spawn: multiple lives
        }
    }
}

/// Status of a game session
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq)]
pub enum GameStatus {
    WaitingForPlayers, // Waiting for players to join
    InProgress,        // Game is active with all players joined
    Completed,         // Game has finished
    Distributed,       // Rewards have been distributed
    Expired,           // Game session has expired
    Cancelled,         // Game was cancelled
}

impl Default for GameStatus {
    fn default() -> Self {
        Self::WaitingForPlayers
    }
}

/// Represents a team in the game
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Default)]
pub struct Team {
    pub players: [Pubkey; MAX_PLAYERS_PER_TEAM], // Array of player public keys
    pub total_bet: u64,                          // Total amount bet by team (in lamports)
    pub player_spawns: [u16; MAX_PLAYERS_PER_TEAM], // Number of spawns remaining for each player
    pub player_kills: [u16; MAX_PLAYERS_PER_TEAM], // Number of kills for each player
}

impl Team {
    /// Finds the first empty slot in the team, if available
    /// SECURITY FIX: Improved error handling using proper enum comparison
    pub fn get_empty_slot(&self, player_count: usize) -> Result<usize> {
        // Validate player_count doesn't exceed maximum
        require!(
            player_count <= MAX_PLAYERS_PER_TEAM,
            WagerError::InvalidPlayerCount
        );

        self.players
            .iter()
            .enumerate()
            .find(|(i, player)| **player == Pubkey::default() && *i < player_count)
            .map(|(i, _)| i)
            .ok_or_else(|| error!(WagerError::TeamIsFull))
    }

    /// Checks if the team is full for the given player count
    pub fn is_full(&self, player_count: usize) -> bool {
        self.get_empty_slot(player_count).is_err()
    }

    /// Gets the number of active players in the team
    pub fn get_active_player_count(&self, max_players: usize) -> usize {
        self.players
            .iter()
            .take(max_players)
            .filter(|&player| *player != Pubkey::default())
            .count()
    }

    /// Validates that a player exists in this team
    pub fn contains_player(&self, player: &Pubkey, max_players: usize) -> bool {
        self.players
            .iter()
            .take(max_players)
            .any(|p| p == player && *p != Pubkey::default())
    }

    /// Gets the index of a player in the team
    pub fn get_player_index(&self, player: &Pubkey, max_players: usize) -> Option<usize> {
        self.players
            .iter()
            .take(max_players)
            .position(|p| p == player && *p != Pubkey::default())
    }

    /// Checks if all players in the team are eliminated (no spawns left)
    pub fn is_eliminated(&self, max_players: usize) -> bool {
        self.players
            .iter()
            .take(max_players)
            .enumerate()
            .filter(|(_, player)| **player != Pubkey::default())
            .all(|(i, _)| self.player_spawns[i] == 0)
    }

    /// Gets total kills for the team
    pub fn get_total_kills(&self, max_players: usize) -> u32 {
        self.player_kills
            .iter()
            .take(max_players)
            .map(|&kills| kills as u32)
            .sum()
    }
}

/// Represents a game session between teams with its own pool
#[account]
pub struct GameSession {
    pub session_id: String,      // Unique identifier for the game
    pub authority: Pubkey,       // Creator of the game session
    pub session_bet: u64,        // Required bet amount per player
    pub game_mode: GameMode,     // Game configuration (1v1, 3v3, 5v5)
    pub team_a: Team,            // First team
    pub team_b: Team,            // Second team
    pub status: GameStatus,      // Current game state
    pub created_at: i64,         // Creation timestamp
    pub expires_at: i64,         // SECURITY FIX: Added expiration timestamp
    pub spawns_per_purchase: u16, // SECURITY FIX: Configurable spawn increment
    pub bump: u8,                // PDA bump
    pub vault_bump: u8,          // Vault PDA bump
    pub vault_token_bump: u8,    // Vault token account PDA bump
}

impl GameSession {
    /// Creates a new game session with proper validation
    pub fn new(
        session_id: String,
        authority: Pubkey,
        session_bet: u64,
        game_mode: GameMode,
        current_time: i64,
        bump: u8,
        vault_bump: u8,
        vault_token_bump: u8,
    ) -> Result<Self> {
        // SECURITY FIX: Validate session ID length
        require!(
            session_id.len() <= MAX_SESSION_ID_LENGTH,
            WagerError::SessionIdTooLong
        );

        // SECURITY FIX: Validate bet amount
        require!(session_bet > 0, WagerError::InvalidBetAmount);

        let expires_at = current_time
            .checked_add(SESSION_TIMEOUT_SECONDS)
            .ok_or(WagerError::ArithmeticError)?;

        Ok(Self {
            session_id,
            authority,
            session_bet,
            game_mode,
            team_a: Team::default(),
            team_b: Team::default(),
            status: GameStatus::WaitingForPlayers,
            created_at: current_time,
            expires_at,
            spawns_per_purchase: DEFAULT_SPAWN_COUNT,
            bump,
            vault_bump,
            vault_token_bump,
        })
    }

    /// SECURITY FIX: Check if session has expired
    pub fn is_expired(&self, current_time: i64) -> bool {
        current_time >= self.expires_at
    }

    /// Gets an empty slot for a player in the specified team
    /// SECURITY FIX: Added expiration check and duplicate player validation
    pub fn get_player_empty_slot(&self, team: u8, current_time: i64) -> Result<usize> {
        // Check if session has expired
        require!(!self.is_expired(current_time), WagerError::GameSessionExpired);

        // Validate game is in correct state
        require!(
            self.status == GameStatus::WaitingForPlayers,
            WagerError::InvalidGameState
        );

        let player_count = self.game_mode.players_per_team();
        match team {
            0 => self.team_a.get_empty_slot(player_count),
            1 => self.team_b.get_empty_slot(player_count),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// SECURITY FIX: Improved team validation logic
    pub fn check_all_filled(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        
        // Direct boolean check instead of error string matching
        let team_a_full = self.team_a.is_full(player_count);
        let team_b_full = self.team_b.is_full(player_count);
        
        Ok(team_a_full && team_b_full)
    }

    /// SECURITY FIX: Added duplicate player validation
    pub fn validate_player_not_joined(&self, player: &Pubkey) -> Result<()> {
        let player_count = self.game_mode.players_per_team();
        
        require!(
            !self.team_a.contains_player(player, player_count) &&
            !self.team_b.contains_player(player, player_count),
            WagerError::PlayerAlreadyJoined
        );
        
        Ok(())
    }

    /// Checks if the game mode supports pay-to-spawn
    pub fn is_pay_to_spawn(&self) -> bool {
        self.game_mode.is_pay_to_spawn()
    }

    /// Gets all active players from both teams
    pub fn get_all_players(&self) -> Vec<Pubkey> {
        let player_count = self.game_mode.players_per_team();
        let mut players = Vec::new();
        
        // Add team A players
        for i in 0..player_count {
            if self.team_a.players[i] != Pubkey::default() {
                players.push(self.team_a.players[i]);
            }
        }
        
        // Add team B players
        for i in 0..player_count {
            if self.team_b.players[i] != Pubkey::default() {
                players.push(self.team_b.players[i]);
            }
        }
        
        players
    }

    /// Gets the team and index of a player
    pub fn get_player_team_and_index(&self, player: Pubkey) -> Result<(u8, usize)> {
        let player_count = self.game_mode.players_per_team();
        
        // Check team A first
        if let Some(index) = self.team_a.get_player_index(&player, player_count) {
            return Ok((0, index));
        }
        
        // Check team B
        if let Some(index) = self.team_b.get_player_index(&player, player_count) {
            return Ok((1, index));
        }
        
        Err(error!(WagerError::PlayerNotFound))
    }

    /// Gets the player index in a specific team
    pub fn get_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        
        match team {
            0 => self.team_a
                .get_player_index(&player, player_count)
                .ok_or(error!(WagerError::PlayerNotFound)),
            1 => self.team_b
                .get_player_index(&player, player_count)
                .ok_or(error!(WagerError::PlayerNotFound)),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// SECURITY FIX: Improved arithmetic with overflow protection
    pub fn get_kills_and_spawns(&self, player_pubkey: Pubkey) -> Result<u16> {
        let (team, index) = self.get_player_team_and_index(player_pubkey)?;
        
        let (kills, spawns) = match team {
            0 => (
                self.team_a.player_kills[index],
                self.team_a.player_spawns[index],
            ),
            1 => (
                self.team_b.player_kills[index],
                self.team_b.player_spawns[index],
            ),
            _ => return Err(error!(WagerError::InvalidTeam)),
        };
        
        // SECURITY FIX: Use checked arithmetic to prevent overflow
        kills
            .checked_add(spawns)
            .ok_or(error!(WagerError::ArithmeticError))
    }

    /// SECURITY FIX: Secure kill recording with bounds checking
    pub fn add_kill(
        &mut self,
        killer_team: u8,
        killer: Pubkey,
        victim_team: u8,
        victim: Pubkey,
    ) -> Result<()> {
        // Validate game state
        require!(
            self.status == GameStatus::InProgress,
            WagerError::GameNotInProgress
        );

        // Prevent self-kills
        require!(killer != victim, WagerError::SelfKillNotAllowed);

        // Get player indices with validation
        let killer_index = self.get_player_index(killer_team, killer)?;
        let victim_index = self.get_player_index(victim_team, victim)?;

        // SECURITY FIX: Validate indices are within bounds
        require!(
            killer_index < MAX_PLAYERS_PER_TEAM,
            WagerError::InvalidPlayerIndex
        );
        require!(
            victim_index < MAX_PLAYERS_PER_TEAM,
            WagerError::InvalidPlayerIndex
        );

        // SECURITY FIX: Record kill with overflow protection
        match killer_team {
            0 => {
                let current_kills = self.team_a.player_kills[killer_index];
                require!(
                    current_kills < u16::MAX,
                    WagerError::KillCountOverflow
                );
                self.team_a.player_kills[killer_index] = current_kills
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            1 => {
                let current_kills = self.team_b.player_kills[killer_index];
                require!(
                    current_kills < u16::MAX,
                    WagerError::KillCountOverflow
                );
                self.team_b.player_kills[killer_index] = current_kills
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        // SECURITY FIX: Decrement spawns with underflow protection
        match victim_team {
            0 => {
                require!(
                    self.team_a.player_spawns[victim_index] > 0,
                    WagerError::PlayerHasNoSpawns
                );
                self.team_a.player_spawns[victim_index] = self.team_a.player_spawns
                    [victim_index]
                    .checked_sub(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            1 => {
                require!(
                    self.team_b.player_spawns[victim_index] > 0,
                    WagerError::PlayerHasNoSpawns
                );
                self.team_b.player_spawns[victim_index] = self.team_b.player_spawns
                    [victim_index]
                    .checked_sub(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        msg!(
            "Kill recorded: {} (team {}) killed {} (team {})",
            killer,
            killer_team,
            victim,
            victim_team
        );

        Ok(())
    }

    /// SECURITY FIX: Configurable spawn addition with overflow protection
    pub fn add_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        // Validate game state
        require!(
            self.status == GameStatus::InProgress || self.status == GameStatus::WaitingForPlayers,
            WagerError::InvalidGameState
        );

        // Validate player index
        require!(
            player_index < MAX_PLAYERS_PER_TEAM,
            WagerError::InvalidPlayerIndex
        );

        // SECURITY FIX: Use configurable spawn increment with overflow protection
        let spawn_increment = self.spawns_per_purchase;

        match team {
            0 => {
                let current_spawns = self.team_a.player_spawns[player_index];
                self.team_a.player_spawns[player_index] = current_spawns
                    .checked_add(spawn_increment)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            1 => {
                let current_spawns = self.team_b.player_spawns[player_index];
                self.team_b.player_spawns[player_index] = current_spawns
                    .checked_add(spawn_increment)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        msg!(
            "Added {} spawns to player {} in team {}",
            spawn_increment,
            player_index,
            team
        );

        Ok(())
    }

    /// Initialize default spawns for a player when they join
    pub fn initialize_player_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        // Validate player index
        require!(
            player_index < MAX_PLAYERS_PER_TEAM,
            WagerError::InvalidPlayerIndex
        );

        let default_spawns = self.game_mode.default_spawn_count();

        match team {
            0 => {
                self.team_a.player_spawns[player_index] = default_spawns;
            }
            1 => {
                self.team_b.player_spawns[player_index] = default_spawns;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        Ok(())
    }

    /// Check if a team has won (opponent team eliminated)
    pub fn check_winner(&self) -> Option<u8> {
        let player_count = self.game_mode.players_per_team();
        
        if self.team_a.is_eliminated(player_count) && !self.team_b.is_eliminated(player_count) {
            Some(1) // Team B wins
        } else if self.team_b.is_eliminated(player_count) && !self.team_a.is_eliminated(player_count) {
            Some(0) // Team A wins
        } else {
            None // No winner yet or tie
        }
    }

    /// Get team statistics
    pub fn get_team_stats(&self, team: u8) -> Result<(u32, u16)> {
        let player_count = self.game_mode.players_per_team();
        
        match team {
            0 => Ok((
                self.team_a.get_total_kills(player_count),
                self.team_a.player_spawns.iter().take(player_count).sum(),
            )),
            1 => Ok((
                self.team_b.get_total_kills(player_count),
                self.team_b.player_spawns.iter().take(player_count).sum(),
            )),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// Validate that the game session can transition to in-progress state
    pub fn can_start(&self) -> Result<bool> {
        require!(
            self.status == GameStatus::WaitingForPlayers,
            WagerError::InvalidGameState
        );
        
        self.check_all_filled()
    }

    /// Update spawn purchase configuration (only by authority)
    pub fn update_spawns_per_purchase(&mut self, new_spawns_per_purchase: u16) -> Result<()> {
        require!(
            new_spawns_per_purchase > 0 && new_spawns_per_purchase <= 50,
            WagerError::InvalidSpawnCount
        );
        
        self.spawns_per_purchase = new_spawns_per_purchase;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_mode_players_per_team() {
        assert_eq!(GameMode::WinnerTakesAllOneVsOne.players_per_team(), 1);
        assert_eq!(GameMode::WinnerTakesAllThreeVsThree.players_per_team(), 3);
        assert_eq!(GameMode::WinnerTakesAllFiveVsFive.players_per_team(), 5);
    }

    #[test]
    fn test_team_validation() {
        let mut team = Team::default();
        assert!(team.get_empty_slot(1).is_ok());
        
        // Fill first slot
        team.players[0] = Pubkey::new_unique();
        assert!(team.get_empty_slot(1).is_err()); // Should be full for 1v1
        assert!(team.get_empty_slot(2).is_ok());  // Should have slot for 2v2+
    }

    #[test]
    fn test_session_expiration() {
        let current_time = 1000;
        let session = GameSession::new(
            "test".to_string(),
            Pubkey::new_unique(),
            100,
            GameMode::WinnerTakesAllOneVsOne,
            current_time,
            1,
            2,
            3,
        ).unwrap();

        assert!(!session.is_expired(current_time));
        assert!(session.is_expired(current_time + SESSION_TIMEOUT_SECONDS + 1));
    }
}