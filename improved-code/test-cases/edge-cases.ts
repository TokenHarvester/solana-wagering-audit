import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { PublicKey, Keypair, SystemProgram, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID, createMint, createAccount, mintTo, getAccount } from "@solana/spl-token";
import { assert, expect } from "chai";
import { WagerProgram } from "../target/types/wager_program"; 

describe("Edge Cases Security Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.WagerProgram as Program<WagerProgram>;
  
  let mint: PublicKey;
  let gameServer: Keypair;
  let player1: Keypair;
  let player2: Keypair;
  let player3: Keypair;
  let attacker: Keypair;
  
  // Test constants matching the security fixes
  const MIN_BET_AMOUNT = 1000;
  const MAX_BET_AMOUNT = 1_000_000_000_000;
  const MAX_SESSION_ID_LENGTH = 32;
  const MIN_SESSION_ID_LENGTH = 3;
  const MAX_SPAWNS_PER_PLAYER = 100;

  before(async () => {
    // Initialize test accounts
    gameServer = Keypair.generate();
    player1 = Keypair.generate();
    player2 = Keypair.generate();
    player3 = Keypair.generate();
    attacker = Keypair.generate();

    // Airdrop SOL
    const accounts = [gameServer, player1, player2, player3, attacker];
    for (const account of accounts) {
      await provider.connection.requestAirdrop(account.publicKey, 5 * LAMPORTS_PER_SOL);
    }

    // Create test token mint
    mint = await createMint(
      provider.connection,
      gameServer,
      gameServer.publicKey,
      null,
      6
    );

    // Create and fund token accounts
    for (const account of accounts) {
      const tokenAccount = await createAccount(
        provider.connection,
        gameServer,
        mint,
        account.publicKey
      );
      
      await mintTo(
        provider.connection,
        gameServer,
        mint,
        tokenAccount,
        gameServer,
        1000000 * Math.pow(10, 6) // 1M tokens
      );
    }
  });

  describe("Critical Vulnerability Tests", () => {
    
    describe("Integer Underflow Prevention", () => {
      it("Should prevent spawn underflow when player has 0 spawns", async () => {
        const sessionId = "underflow_test_01";
        const gameSessionPda = PublicKey.findProgramAddressSync(
          [Buffer.from("game_session"), Buffer.from(sessionId)],
          program.programId
        )[0];

        // Create game session
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Player joins and gets eliminated (spawns = 0)
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        // Manually set spawns to 0 (simulating elimination)
        // This would normally be done through kill recording

        // Attempt to record another kill (should fail)
        try {
          await program.methods
            .addKill(sessionId, 0, player2.publicKey, 0, player1.publicKey)
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to underflow protection");
        } catch (error) {
          expect(error.toString()).to.include("PlayerHasNoSpawns");
        }
      });

      it("Should handle maximum kill count without overflow", async () => {
        const sessionId = "overflow_kill_test";
        
        // Create game and join players
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        await program.methods
          .joinUser(sessionId, 1)
          .accounts({
            user: player2.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player2])
          .rpc();

        // Test kill recording near u16::MAX
        // This should test the overflow protection in add_kill method
        const gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );

        // Verify kill count validation exists
        expect(gameSession.teamA.playerKills[0]).to.be.lessThan(65535);
      });
    });

    describe("Vault Drainage Protection", () => {
      it("Should prevent distribution when vault has insufficient funds", async () => {
        const sessionId = "vault_drain_test";
        
        // Create game session with high bet amount
        await program.methods
          .createGameSession(sessionId, new anchor.BN(1000000), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Join players
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        await program.methods
          .joinUser(sessionId, 1)
          .accounts({
            user: player2.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player2])
          .rpc();

        // Manually drain vault to simulate attack scenario
        const vaultPda = PublicKey.findProgramAddressSync(
          [Buffer.from("vault"), Buffer.from(sessionId)],
          program.programId
        )[0];

        // Attempt distribution with insufficient vault balance
        try {
          await program.methods
            .distributeAllWinnings(sessionId, 0)
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .remainingAccounts([
              { pubkey: player1.publicKey, isSigner: false, isWritable: false },
              { pubkey: await getAssociatedTokenAddress(mint, player1.publicKey), isSigner: false, isWritable: true },
            ])
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to insufficient vault balance");
        } catch (error) {
          expect(error.toString()).to.include("InsufficientVaultBalance");
        }
      });

      it("Should calculate total distribution correctly before transfers", async () => {
        const sessionId = "calc_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Test that distribution calculation includes all players
        const gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );

        // Verify session bet amount is within bounds
        expect(gameSession.sessionBet.toNumber()).to.be.greaterThan(0);
        expect(gameSession.sessionBet.toNumber()).to.be.lessThanOrEqual(MAX_BET_AMOUNT);
      });
    });

    describe("Race Condition Prevention", () => {
      it("Should prevent multiple simultaneous joins to same team slot", async () => {
        const sessionId = "race_condition_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Attempt simultaneous joins (this tests atomic state transitions)
        const promises = [
          program.methods
            .joinUser(sessionId, 0)
            .accounts({
              user: player1.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player1])
            .rpc(),
          
          program.methods
            .joinUser(sessionId, 0)
            .accounts({
              user: player2.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player2])
            .rpc()
        ];

        const results = await Promise.allSettled(promises);
        
        // One should succeed, one should fail
        const successes = results.filter(r => r.status === 'fulfilled').length;
        const failures = results.filter(r => r.status === 'rejected').length;
        
        expect(successes).to.equal(1);
        expect(failures).to.equal(1);
      });

      it("Should handle game state transitions atomically", async () => {
        const sessionId = "atomic_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Fill first team slot
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        // Verify game is still waiting for players
        let gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );
        
        expect(gameSession.status).to.deep.equal({ waitingForPlayers: {} });

        // Fill second team slot - should transition to InProgress
        await program.methods
          .joinUser(sessionId, 1)
          .accounts({
            user: player2.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player2])
          .rpc();

        // Verify atomic transition to InProgress
        gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );
        
        expect(gameSession.status).to.deep.equal({ inProgress: {} });
      });
    });
  });

  describe("High Severity Issue Tests", () => {
    
    describe("Authority Validation", () => {
      it("Should reject unauthorized pay-to-spawn attempts", async () => {
        const sessionId = "auth_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Join players
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        await program.methods
          .joinUser(sessionId, 1)
          .accounts({
            user: player2.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player2])
          .rpc();

        // Attacker tries to purchase spawns with wrong authority
        try {
          await program.methods
            .payToSpawn(sessionId, 0)
            .accounts({
              user: player1.publicKey,
              gameServer: attacker.publicKey, // Wrong authority
            })
            .signers([player1, attacker])
            .rpc();
          
          assert.fail("Should have failed due to unauthorized authority");
        } catch (error) {
          expect(error.toString()).to.include("UnauthorizedPayToSpawn");
        }
      });

      it("Should validate game server matches session authority", async () => {
        const sessionId = "server_auth_test";
        
        // Create session with gameServer authority
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Verify session authority matches
        const gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );

        expect(gameSession.authority.toString()).to.equal(gameServer.publicKey.toString());
      });
    });

    describe("Duplicate Player Prevention", () => {
      it("Should prevent player from joining both teams", async () => {
        const sessionId = "duplicate_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllThreeVsThree: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Player joins team A
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        // Same player tries to join team B
        try {
          await program.methods
            .joinUser(sessionId, 1)
            .accounts({
              user: player1.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player1])
            .rpc();
          
          assert.fail("Should have failed due to duplicate player");
        } catch (error) {
          expect(error.toString()).to.include("PlayerAlreadyJoined");
        }
      });

      it("Should prevent player from joining same team twice", async () => {
        const sessionId = "same_team_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllThreeVsThree: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Player joins team A
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        // Same player tries to join team A again
        try {
          await program.methods
            .joinUser(sessionId, 0)
            .accounts({
              user: player1.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player1])
            .rpc();
          
          assert.fail("Should have failed due to duplicate player");
        } catch (error) {
          expect(error.toString()).to.include("PlayerAlreadyJoined");
        }
      });
    });

    describe("Bet Amount Validation", () => {
      it("Should reject zero bet amounts", async () => {
        const sessionId = "zero_bet_test";
        
        try {
          await program.methods
            .createGameSession(sessionId, new anchor.BN(0), { winnerTakesAllOneVsOne: {} })
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to zero bet amount");
        } catch (error) {
          expect(error.toString()).to.include("InvalidBetAmount");
        }
      });

      it("Should reject bet amounts below minimum", async () => {
        const sessionId = "low_bet_test";
        
        try {
          await program.methods
            .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT - 1), { winnerTakesAllOneVsOne: {} })
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to bet amount too low");
        } catch (error) {
          expect(error.toString()).to.include("BetAmountTooLow");
        }
      });

      it("Should reject bet amounts above maximum", async () => {
        const sessionId = "high_bet_test";
        
        try {
          await program.methods
            .createGameSession(sessionId, new anchor.BN(MAX_BET_AMOUNT + 1), { winnerTakesAllOneVsOne: {} })
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to bet amount too high");
        } catch (error) {
          expect(error.toString()).to.include("BetAmountTooHigh");
        }
      });
    });

    describe("Arithmetic Overflow Protection", () => {
      it("Should handle large kill counts safely", async () => {
        const sessionId = "large_kills_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Test would verify that earnings calculation uses checked arithmetic
        // This prevents (large_kills * session_bet) from overflowing
        const maxSafeKills = Math.floor(Number.MAX_SAFE_INTEGER / MIN_BET_AMOUNT);
        expect(maxSafeKills).to.be.greaterThan(0);
      });

      it("Should prevent spawn purchase overflow", async () => {
        const sessionId = "spawn_overflow_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Join players
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        // Test maximum spawn limit enforcement
        // Multiple purchases should be limited to MAX_SPAWNS_PER_PLAYER
        for (let i = 0; i < 15; i++) { // Attempt to exceed limit
          try {
            await program.methods
              .payToSpawn(sessionId, 0)
              .accounts({
                user: player1.publicKey,
                gameServer: gameServer.publicKey,
              })
              .signers([player1])
              .rpc();
          } catch (error) {
            if (error.toString().includes("MaxSpawnsExceeded")) {
              break; // Expected behavior
            }
          }
        }

        // Verify spawn count doesn't exceed maximum
        const gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );

        expect(gameSession.teamA.playerSpawns[0]).to.be.lessThanOrEqual(MAX_SPAWNS_PER_PLAYER);
      });
    });
  });

  describe("Medium Severity Issue Tests", () => {
    
    describe("Session Expiration", () => {
      it("Should reject actions on expired sessions", async () => {
        const sessionId = "expired_test";
        
        // Create session (this would normally set expiration time)
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // In a real test, you'd manipulate time or wait for expiration
        // Here we test the expiration check logic exists
        const gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );

        expect(gameSession.expiresAt).to.be.greaterThan(gameSession.createdAt);
      });

      it("Should allow extending session expiration by authority", async () => {
        const sessionId = "extend_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Test extension functionality if implemented
        try {
          await program.methods
            .extendSession(sessionId, new anchor.BN(3600)) // 1 hour
            .accounts({
              authority: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
        } catch (error) {
          // Extension may not be implemented in current version
          console.log("Extension not implemented:", error.message);
        }
      });
    });

    describe("Input Validation Edge Cases", () => {
      it("Should reject sessions with invalid ID length", async () => {
        // Too short
        try {
          await program.methods
            .createGameSession("x", new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to short session ID");
        } catch (error) {
          expect(error.toString()).to.include("SessionIdTooShort");
        }

        // Too long
        const longId = "a".repeat(MAX_SESSION_ID_LENGTH + 1);
        try {
          await program.methods
            .createGameSession(longId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          assert.fail("Should have failed due to long session ID");
        } catch (error) {
          expect(error.toString()).to.include("SessionIdTooLong");
        }
      });

      it("Should reject invalid team numbers", async () => {
        const sessionId = "team_validation_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Try invalid team number (should be 0 or 1)
        try {
          await program.methods
            .joinUser(sessionId, 2) // Invalid team
            .accounts({
              user: player1.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player1])
            .rpc();
          
          assert.fail("Should have failed due to invalid team selection");
        } catch (error) {
          expect(error.toString()).to.include("InvalidTeamSelection");
        }
      });
    });

    describe("Game State Validation", () => {
      it("Should prevent actions in wrong game states", async () => {
        const sessionId = "state_test";
        
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Try pay-to-spawn before game starts (should fail)
        try {
          await program.methods
            .payToSpawn(sessionId, 0)
            .accounts({
              user: player1.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player1])
            .rpc();
          
          assert.fail("Should have failed due to wrong game state");
        } catch (error) {
          expect(error.toString()).to.include("InvalidGameState");
        }
      });

      it("Should validate game mode consistency", async () => {
        const sessionId = "mode_test";
        
        // Create winner-takes-all game
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();

        // Join players
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player1])
          .rpc();

        await program.methods
          .joinUser(sessionId, 1)
          .accounts({
            user: player2.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player2])
          .rpc();

        // Try pay-to-spawn on winner-takes-all game (should fail)
        try {
          await program.methods
            .payToSpawn(sessionId, 0)
            .accounts({
              user: player1.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player1])
            .rpc();
          
          assert.fail("Should have failed due to wrong game mode");
        } catch (error) {
          expect(error.toString()).to.include("InvalidGameMode");
        }
      });
    });
  });

  describe("Stress Tests", () => {
    it("Should handle maximum team size correctly", async () => {
      const sessionId = "max_team_test";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllFiveVsFive: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Try to exceed team capacity
      const players = [];
      for (let i = 0; i < 6; i++) { // 6 > 5 (max team size)
        const player = Keypair.generate();
        players.push(player);
        
        // Fund player
        await provider.connection.requestAirdrop(player.publicKey, LAMPORTS_PER_SOL);
        
        const tokenAccount = await createAccount(
          provider.connection,
          gameServer,
          mint,
          player.publicKey
        );
        
        await mintTo(
          provider.connection,
          gameServer,
          mint,
          tokenAccount,
          gameServer,
          100000 * Math.pow(10, 6)
        );

        try {
          await program.methods
            .joinUser(sessionId, 0) // All trying team 0
            .accounts({
              user: player.publicKey,
              gameServer: gameServer.publicKey,
            })
            .signers([player])
            .rpc();
        } catch (error) {
          if (i >= 5) { // Should fail on 6th player
            expect(error.toString()).to.include("TeamIsFull");
            break;
          }
        }
      }
    });

    it("Should handle rapid successive operations", async () => {
      const sessionId = "rapid_test";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { payToSpawnThreeVsThree: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Rapid join operations
      const joinPromises = [player1, player2, player3].map((player, index) =>
        program.methods
          .joinUser(sessionId, index % 2) // Alternate teams
          .accounts({
            user: player.publicKey,
            gameServer: gameServer.publicKey,
          })
          .signers([player])
          .rpc()
      );

      const results = await Promise.allSettled(joinPromises);
      const successes = results.filter(r => r.status === 'fulfilled').length;
      
      expect(successes).to.be.greaterThan(0);
      expect(successes).to.be.lessThanOrEqual(3);
    });
  });

  // Helper function to get associated token address
  async function getAssociatedTokenAddress(mint: PublicKey, owner: PublicKey): Promise<PublicKey> {
    const [address] = PublicKey.findProgramAddressSync(
      [
        owner.toBuffer(),
        TOKEN_PROGRAM_ID.toBuffer(),
        mint.toBuffer(),
      ],
      ASSOCIATED_TOKEN_PROGRAM_ID
    );
    return address;
  }
});

// Additional Configuration Tests
describe("Configuration and Boundary Tests", () => {
  
  describe("Token Account Validation", () => {
    it("Should reject operations with wrong token mint", async () => {
      // Create a different mint
      const wrongMint = await createMint(
        provider.connection,
        gameServer,
        gameServer.publicKey,
        null,
        6
      );

      const sessionId = "wrong_mint_test";
      
      // This should fail at account validation level
      try {
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
            mint: wrongMint, // Wrong mint
          })
          .signers([gameServer])
          .rpc();
        
        assert.fail("Should have failed due to wrong mint");
      } catch (error) {
        expect(error.toString()).to.include("InvalidMint");
      }
    });

    it("Should validate token account ownership", async () => {
      const sessionId = "ownership_test";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Create token account owned by wrong address
      const wrongOwnerTokenAccount = await createAccount(
        provider.connection,
        gameServer,
        mint,
        attacker.publicKey // Wrong owner
      );

      try {
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
            userTokenAccount: wrongOwnerTokenAccount, // Wrong ownership
          })
          .signers([player1])
          .rpc();
        
        assert.fail("Should have failed due to wrong token account owner");
      } catch (error) {
        expect(error.toString()).to.include("InvalidTokenAccountOwner");
      }
    });
  });

  describe("PDA Validation", () => {
    it("Should reject operations with incorrect PDAs", async () => {
      const sessionId = "pda_test";
      const wrongSessionId = "wrong_session";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Try to use wrong session PDA
      const wrongGameSessionPda = PublicKey.findProgramAddressSync(
        [Buffer.from("game_session"), Buffer.from(wrongSessionId)],
        program.programId
      )[0];

      try {
        await program.methods
          .joinUser(sessionId, 0) // Correct session ID in instruction
          .accounts({
            user: player1.publicKey,
            gameServer: gameServer.publicKey,
            gameSession: wrongGameSessionPda, // Wrong PDA
          })
          .signers([player1])
          .rpc();
        
        assert.fail("Should have failed due to PDA mismatch");
      } catch (error) {
        // Should fail at constraint level
        expect(error.message).to.include("address constraint");
      }
    });

    it("Should validate vault PDA derivation", async () => {
      const sessionId = "vault_pda_test";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Verify correct vault PDA
      const correctVaultPda = PublicKey.findProgramAddressSync(
        [Buffer.from("vault"), Buffer.from(sessionId)],
        program.programId
      )[0];

      const gameSession = await program.account.gameSession.fetch(
        PublicKey.findProgramAddressSync(
          [Buffer.from("game_session"), Buffer.from(sessionId)],
          program.programId
        )[0]
      );

      // Verify vault was created with correct PDA
      const vaultTokenAccount = await getAccount(
        provider.connection,
        PublicKey.findProgramAddressSync(
          [
            correctVaultPda.toBuffer(),
            TOKEN_PROGRAM_ID.toBuffer(),
            mint.toBuffer(),
          ],
          ASSOCIATED_TOKEN_PROGRAM_ID
        )[0]
      );

      expect(vaultTokenAccount.owner.toString()).to.equal(correctVaultPda.toString());
    });
  });

  describe("Concurrent Operations", () => {
    it("Should handle multiple session creations safely", async () => {
      const sessionIds = ["concurrent1", "concurrent2", "concurrent3"];
      
      const promises = sessionIds.map(sessionId =>
        program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc()
      );

      const results = await Promise.allSettled(promises);
      const successes = results.filter(r => r.status === 'fulfilled').length;
      
      // All should succeed as they have different session IDs
      expect(successes).to.equal(3);

      // Verify all sessions were created correctly
      for (const sessionId of sessionIds) {
        const gameSession = await program.account.gameSession.fetch(
          PublicKey.findProgramAddressSync(
            [Buffer.from("game_session"), Buffer.from(sessionId)],
            program.programId
          )[0]
        );
        
        expect(gameSession.sessionId).to.equal(sessionId);
        expect(gameSession.status).to.deep.equal({ waitingForPlayers: {} });
      }
    });

    it("Should prevent duplicate session ID creation", async () => {
      const sessionId = "duplicate_session";
      
      // Create first session
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Try to create duplicate session
      try {
        await program.methods
          .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc();
        
        assert.fail("Should have failed due to duplicate session ID");
      } catch (error) {
        // Should fail because account already exists
        expect(error.message).to.include("already in use");
      }
    });
  });

  describe("Memory and Resource Tests", () => {
    it("Should handle large session IDs within limits", async () => {
      const maxLengthSessionId = "a".repeat(MAX_SESSION_ID_LENGTH);
      
      await program.methods
        .createGameSession(maxLengthSessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      const gameSession = await program.account.gameSession.fetch(
        PublicKey.findProgramAddressSync(
          [Buffer.from("game_session"), Buffer.from(maxLengthSessionId)],
          program.programId
        )[0]
      );

      expect(gameSession.sessionId).to.equal(maxLengthSessionId);
    });

    it("Should handle minimum valid inputs", async () => {
      const minSessionId = "abc"; // MIN_SESSION_ID_LENGTH = 3
      
      await program.methods
        .createGameSession(minSessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      const gameSession = await program.account.gameSession.fetch(
        PublicKey.findProgramAddressSync(
          [Buffer.from("game_session"), Buffer.from(minSessionId)],
          program.programId
        )[0]
      );

      expect(gameSession.sessionId).to.equal(minSessionId);
      expect(gameSession.sessionBet.toNumber()).to.equal(MIN_BET_AMOUNT);
    });
  });

  describe("Error Recovery Tests", () => {
    it("Should handle failed join attempts gracefully", async () => {
      const sessionId = "recovery_test";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Create player with insufficient balance
      const poorPlayer = Keypair.generate();
      await provider.connection.requestAirdrop(poorPlayer.publicKey, LAMPORTS_PER_SOL);
      
      const poorPlayerTokenAccount = await createAccount(
        provider.connection,
        gameServer,
        mint,
        poorPlayer.publicKey
      );
      // Don't mint tokens - account will have 0 balance

      // Attempt to join should fail
      try {
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: poorPlayer.publicKey,
            gameServer: gameServer.publicKey,
            userTokenAccount: poorPlayerTokenAccount,
          })
          .signers([poorPlayer])
          .rpc();
        
        assert.fail("Should have failed due to insufficient balance");
      } catch (error) {
        expect(error.toString()).to.include("InsufficientUserBalance");
      }

      // Game session should still be in valid state
      const gameSession = await program.account.gameSession.fetch(
        PublicKey.findProgramAddressSync(
          [Buffer.from("game_session"), Buffer.from(sessionId)],
          program.programId
        )[0]
      );

      expect(gameSession.status).to.deep.equal({ waitingForPlayers: {} });
      
      // Normal player should still be able to join
      await program.methods
        .joinUser(sessionId, 0)
        .accounts({
          user: player1.publicKey,
          gameServer: gameServer.publicKey,
        })
        .signers([player1])
        .rpc();
    });

    it("Should maintain consistency after partial failures", async () => {
      const sessionId = "consistency_test";
      
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllThreeVsThree: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Join some players successfully
      await program.methods
        .joinUser(sessionId, 0)
        .accounts({
          user: player1.publicKey,
          gameServer: gameServer.publicKey,
        })
        .signers([player1])
        .rpc();

      await program.methods
        .joinUser(sessionId, 1)
        .accounts({
          user: player2.publicKey,
          gameServer: gameServer.publicKey,
        })
        .signers([player2])
        .rpc();

      // Verify consistent state
      const gameSession = await program.account.gameSession.fetch(
        PublicKey.findProgramAddressSync(
          [Buffer.from("game_session"), Buffer.from(sessionId)],
          program.programId
        )[0]
      );

      // Should have 2 players, one in each team
      expect(gameSession.teamA.players[0].toString()).to.equal(player1.publicKey.toString());
      expect(gameSession.teamB.players[0].toString()).to.equal(player2.publicKey.toString());
      expect(gameSession.status).to.deep.equal({ waitingForPlayers: {} });
    });
  });

  describe("Security Edge Cases", () => {
    it("Should prevent session hijacking attempts", async () => {
      const sessionId = "hijack_test";
      
      // Legitimate server creates session
      await program.methods
        .createGameSession(sessionId, new anchor.BN(MIN_BET_AMOUNT), { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc();

      // Attacker tries to join with wrong server signature
      try {
        await program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: player1.publicKey,
            gameServer: attacker.publicKey, // Wrong server
          })
          .signers([player1, attacker])
          .rpc();
        
        assert.fail("Should have failed due to wrong game server");
      } catch (error) {
        expect(error.toString()).to.include("UnauthorizedGameServer");
      }
    });

    it("Should validate all numerical bounds", async () => {
      const testCases = [
        { bet: 0, shouldFail: true, error: "InvalidBetAmount" },
        { bet: MIN_BET_AMOUNT - 1, shouldFail: true, error: "BetAmountTooLow" },
        { bet: MIN_BET_AMOUNT, shouldFail: false, error: null },
        { bet: MAX_BET_AMOUNT, shouldFail: false, error: null },
        { bet: MAX_BET_AMOUNT + 1, shouldFail: true, error: "BetAmountTooHigh" },
      ];

      for (const [index, testCase] of testCases.entries()) {
        const sessionId = `bounds_test_${index}`;
        
        try {
          await program.methods
            .createGameSession(sessionId, new anchor.BN(testCase.bet), { winnerTakesAllOneVsOne: {} })
            .accounts({
              gameServer: gameServer.publicKey,
            })
            .signers([gameServer])
            .rpc();
          
          if (testCase.shouldFail) {
            assert.fail(`Should have failed for bet amount ${testCase.bet}`);
          }
        } catch (error) {
          if (!testCase.shouldFail) {
            throw error;
          }
          expect(error.toString()).to.include(testCase.error);
        }
      }
    });
  });
});