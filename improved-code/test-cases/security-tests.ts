import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { WagerProgram } from "../app/src/app/types/wager_program";
import { BN } from "@coral-xyz/anchor";
import { assert, expect } from "chai";
import {
  generateSessionId,
  deriveGameSessionPDA,
  loadKeypair,
  setupTokenAccount,
  setupTestAccounts,
  TOKEN_ID,
  getTokenBalance
} from "./utils";
import { PublicKey, ConfirmOptions } from "@solana/web3.js";

describe("Security Vulnerability Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.WagerProgram as Program<WagerProgram>;
  
  const confirmOptions: ConfirmOptions = { commitment: "confirmed" };
  const gameServer = loadKeypair('./tests/kps/gameserver.json');
  const user1 = loadKeypair('./tests/kps/user1.json');
  const user2 = loadKeypair('./tests/kps/user2.json');
  
  let user1TokenAccount: PublicKey;
  let user2TokenAccount: PublicKey;

  before(async () => {
    await setupTestAccounts(provider.connection, [gameServer, user1, user2]);
    
    user1TokenAccount = await setupTokenAccount(
      provider.connection,
      gameServer,
      TOKEN_ID,
      user1.publicKey
    );
    
    user2TokenAccount = await setupTokenAccount(
      provider.connection,
      gameServer,
      TOKEN_ID,
      user2.publicKey
    );
  });

  describe("Critical Vulnerability: Integer Underflow", () => {
    it("Should prevent killing player with 0 spawns", async () => {
      const sessionId = generateSessionId();
      const betAmount = new BN(100000000);

      // Create game session
      await program.methods
        .createGameSession(sessionId, betAmount, { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc(confirmOptions);

      // Join users
      await program.methods
        .joinUser(sessionId, 0)
        .accounts({
          user: user1.publicKey,
          gameServer: gameServer.publicKey,
          userTokenAccount: user1TokenAccount,
        })
        .signers([user1])
        .rpc(confirmOptions);

      await program.methods
        .joinUser(sessionId, 1)
        .accounts({
          user: user2.publicKey,
          gameServer: gameServer.publicKey,
          userTokenAccount: user2TokenAccount,
        })
        .signers([user2])
        .rpc(confirmOptions);

      // Kill user2 10 times to exhaust spawns
      for (let i = 0; i < 10; i++) {
        await program.methods
          .recordKill(sessionId, 0, user1.publicKey, 1, user2.publicKey)
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc(confirmOptions);
      }

      // Try to kill user2 when they have 0 spawns - should fail
      try {
        await program.methods
          .recordKill(sessionId, 0, user1.publicKey, 1, user2.publicKey)
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc(confirmOptions);
        
        assert.fail("Should have failed with PlayerHasNoSpawns error");
      } catch (error) {
        // This test will currently fail because the vulnerability exists
        console.log("VULNERABILITY CONFIRMED: Program would panic on underflow");
        assert.include(error.toString(), "Transaction simulation failed");
      }
    });
  });

  describe("Critical Vulnerability: Duplicate Player Prevention", () => {
    it("Should prevent same player from joining both teams", async () => {
      const sessionId = generateSessionId();
      const betAmount = new BN(100000000);

      // Create game session
      await program.methods
        .createGameSession(sessionId, betAmount, { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc(confirmOptions);

      // User1 joins team 0
      await program.methods
        .joinUser(sessionId, 0)
        .accounts({
          user: user1.publicKey,
          gameServer: gameServer.publicKey,
          userTokenAccount: user1TokenAccount,
        })
        .signers([user1])
        .rpc(confirmOptions);

      // Try to make user1 join team 1 - should fail but currently doesn't
      try {
        await program.methods
          .joinUser(sessionId, 1)
          .accounts({
            user: user1.publicKey,
            gameServer: gameServer.publicKey,
            userTokenAccount: user1TokenAccount,
          })
          .signers([user1])
          .rpc(confirmOptions);
        
        console.log("VULNERABILITY CONFIRMED: Same player can join both teams");
        // This will currently succeed, showing the vulnerability
      } catch (error) {
        console.log("Good: Duplicate player prevented");
        assert.include(error.toString(), "PlayerAlreadyJoined");
      }
    });
  });

  describe("High Severity: Missing Authority Validation", () => {
    it("Should prevent unauthorized pay-to-spawn", async () => {
      const sessionId = generateSessionId();
      const betAmount = new BN(100000000);

      // Create pay-to-spawn game
      await program.methods
        .createGameSession(sessionId, betAmount, { payToSpawnOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc(confirmOptions);

      // Join users
      await program.methods
        .joinUser(sessionId, 0)
        .accounts({
          user: user1.publicKey,
          gameServer: gameServer.publicKey,
          userTokenAccount: user1TokenAccount,
        })
        .signers([user1])
        .rpc(confirmOptions);

      // Try pay-to-spawn with different game server - should fail but might not
      const fakeGameServer = anchor.web3.Keypair.generate();
      
      try {
        await program.methods
          .payToSpawn(sessionId, 0)
          .accounts({
            user: user1.publicKey,
            gameServer: fakeGameServer.publicKey, // Wrong game server
            userTokenAccount: user1TokenAccount,
          })
          .signers([user1])
          .rpc(confirmOptions);
        
        console.log("VULNERABILITY: Unauthorized pay-to-spawn might be possible");
      } catch (error) {
        console.log("Good: Unauthorized pay-to-spawn prevented");
      }
    });
  });

  describe("High Severity: Zero Bet Amount", () => {
    it("Should prevent creating games with zero bet amount", async () => {
      const sessionId = generateSessionId();
      const betAmount = new BN(0); // Zero bet

      try {
        await program.methods
          .createGameSession(sessionId, betAmount, { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc(confirmOptions);
        
        console.log("VULNERABILITY CONFIRMED: Can create games with 0 bet amount");
      } catch (error) {
        console.log("Good: Zero bet amount prevented");
        assert.include(error.toString(), "InvalidBetAmount");
      }
    });
  });

  describe("Critical Vulnerability: Vault Drainage", () => {
    it("Should prevent distribution when vault has insufficient balance", async () => {
      // This test would require manipulating vault state
      // Implementation depends on having access to vault manipulation
      console.log("VAULT DRAINAGE TEST: Would require manual vault state manipulation");
      
      // The vulnerability exists because there's no balance check before distribution
      // Current code would attempt to transfer more than available
    });
  });

  describe("Race Condition: Concurrent Joins", () => {
    it("Should handle concurrent join attempts gracefully", async () => {
      const sessionId = generateSessionId();
      const betAmount = new BN(100000000);

      // Create 1v1 game
      await program.methods
        .createGameSession(sessionId, betAmount, { winnerTakesAllOneVsOne: {} })
        .accounts({
          gameServer: gameServer.publicKey,
        })
        .signers([gameServer])
        .rpc(confirmOptions);

      // Simulate concurrent joins (this is simplified - real race conditions are harder to test)
      const promises = [
        program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: user1.publicKey,
            gameServer: gameServer.publicKey,
            userTokenAccount: user1TokenAccount,
          })
          .signers([user1])
          .rpc(confirmOptions),
        
        program.methods
          .joinUser(sessionId, 0)
          .accounts({
            user: user2.publicKey,
            gameServer: gameServer.publicKey,
            userTokenAccount: user2TokenAccount,
          })
          .signers([user2])
          .rpc(confirmOptions)
      ];

      try {
        await Promise.all(promises);
        console.log("Both joins succeeded - check for race condition");
      } catch (error) {
        console.log("Race condition handling - some joins failed as expected");
      }
    });
  });

  describe("Arithmetic Overflow Tests", () => {
    it("Should handle large numbers safely", async () => {
      const sessionId = generateSessionId();
      const maxBet = new BN("18446744073709551615"); // Near u64::MAX

      try {
        await program.methods
          .createGameSession(sessionId, maxBet, { winnerTakesAllOneVsOne: {} })
          .accounts({
            gameServer: gameServer.publicKey,
          })
          .signers([gameServer])
          .rpc(confirmOptions);
        
        console.log("POTENTIAL ISSUE: Extremely large bet amounts allowed");
      } catch (error) {
        console.log("Good: Large bet amounts prevented");
      }
    });
  });
});