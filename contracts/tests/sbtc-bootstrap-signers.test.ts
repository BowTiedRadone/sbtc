import {
  constructMultisigAddress,
  currentSignerAddr,
  deployer,
  errors,
  randomPublicKeys,
  registry,
  signers,
} from "./helpers";
import { test, expect, describe } from "vitest";
import { txOk, txErr, rov } from "@clarigen/test";
import { hex } from "@scure/base";
import {
  AddressHashMode,
  AddressVersion,
  addressFromPublicKeys,
  addressToString,
  pubKeyfromPrivKey,
  serializePublicKey,
} from "@stacks/transactions";
import { p2ms, p2sh } from "@scure/btc-signer";
import { b58ToC32 } from "c32check";

describe("sBTC bootstrap signers contract", () => {
  describe("Rotate keys tests", () => {
    test("Rotate keys wrapper correctly", () => {
      const newKeys = randomPublicKeys(2);
      const receipt = txOk(
        signers.rotateKeysWrapper({
          newKeys,
          newAggregatePubkey: new Uint8Array(33).fill(0),
        }),
        deployer
      );
      expect(receipt.value).toEqual(true);

      const setAggKey = rov(registry.getCurrentAggregatePubkey());
      expect(setAggKey).toEqual(new Uint8Array(33).fill(0));

      expect(rov(registry.getCurrentSignerSet())).toStrictEqual(newKeys);

      const expectedPrincipal = constructMultisigAddress(
        newKeys,
        signers.constants.signatureThreshold
      );
      expect(currentSignerAddr()).toEqual(expectedPrincipal);

      expect(rov(registry.getCurrentSignerData())).toStrictEqual({
        currentAggregatePubkey: new Uint8Array(33).fill(0),
        currentSignerSet: newKeys,
        currentSignerPrincipal: expectedPrincipal,
      });
    });

    test("Rotate keys wrapper incorrect signer key size", () => {
      const receipt = txErr(
        signers.rotateKeysWrapper({
          newKeys: [new Uint8Array(33).fill(0), new Uint8Array(31).fill(0)],
          newAggregatePubkey: new Uint8Array(33).fill(0),
        }),
        currentSignerAddr()
      );
      expect(receipt.value).toEqual(errors.signers.ERR_KEY_SIZE_PREFIX + 11n);
    });

    test("Rotate keys wrapper incorrect aggregate key size", () => {
      const receipt = txErr(
        signers.rotateKeysWrapper({
          newKeys: [new Uint8Array(33).fill(0), new Uint8Array(33).fill(0)],
          newAggregatePubkey: new Uint8Array(31).fill(0),
        }),
        currentSignerAddr()
      );
      expect(receipt.value).toEqual(errors.signers.ERR_KEY_SIZE);
    });
  });

  describe("Constructing a multisig from a list of keys", () => {
    describe("constructing a multisig from two fixed keys", () => {
      const stacksPubkeys = [
        pubKeyfromPrivKey(
          "6d430bb91222408e7706c9001cfaeb91b08c2be6d5ac95779ab52c6b431950e001"
        ),
        pubKeyfromPrivKey(
          "530d9f61984c888536871c6573073bdfc0058896dc1adfe9a6a10dfacadc209101"
        ),
      ];
      const pubkeys = stacksPubkeys.map((pk) => serializePublicKey(pk));

      test("principal created is the same as stacks.js", () => {
        const addr = rov(signers.pubkeysToPrincipal(pubkeys, 2));
        const stacksJsAddr = addressToString(
          addressFromPublicKeys(
            AddressVersion.TestnetMultiSig,
            AddressHashMode.SerializeP2SH,
            2,
            stacksPubkeys
          )
        );
        expect(addr).toEqual(stacksJsAddr);
      });

      // In this example, use yet another library to construct a p2sh ms
      // Bitcoin address, and then convert that to a c32 (stacks) address.
      test("principal is the same as a b58 bitcoin address", () => {
        const btcPayment = p2sh(p2ms(2, pubkeys));
        const c32Addr = b58ToC32(btcPayment.address!, 0x15);
        const addr = rov(signers.pubkeysToPrincipal(pubkeys, 2));
        expect(addr).toEqual(c32Addr);
      });
    });

    test("matches a rust-based fixture for generating script hash", () => {
      // Using this fixture: https://github.com/stacks-network/stacks-core/blob/fa950324fbeea1b5de24dc9c0707b272bb5d7dd8/stacks-common/src/address/mod.rs#L245
      const pubkeys = [
        hex.decode(
          "040fadbbcea0ff3b05f03195b41cd991d7a0af8bd38559943aec99cbdaf0b22cc806b9a4f07579934774cc0c155e781d45c989f94336765e88a66d91cfb9f060b0"
        ),
        hex.decode(
          "04c77f262dda02580d65c9069a8a34c56bd77325bba4110b693b90216f5a3edc0bebc8ce28d61aa86b414aa91ecb29823b11aeed06098fcd97fee4bc73d54b1e96"
        ),
      ];

      const scriptHash = rov(signers.pubkeysToHash(pubkeys, 2));
      expect(scriptHash).toEqual(
        hex.decode("fd3a5e9f5ba311ce6122765f0af8da7488e25d3a")
      );
    });

    describe("Testing multisig computation with random data", () => {
      // This test generates random public keys from 1-1 to 15-15, and all
      // combinations of m-of-n multisig addresses. It then compares the
      // principal generated by the contract to the principal generated by
      // stacks.js.
      test("matching contract code to stacks.js", async () => {
        for (let n = 1; n <= 15; n++) {
          const pubkeys = randomPublicKeys(n);
          for (let m = 1; m <= 15; m++) {
            if (m > n) continue;
            const stacksJsPrincipal = constructMultisigAddress(pubkeys, m);
            const contractPrincipal = rov(
              signers.pubkeysToPrincipal(pubkeys, m)
            );
            expect(contractPrincipal).toEqual(stacksJsPrincipal);
          }
        }
      });

      test("matching multisig compared to @scure/btc-signer", () => {
        function principalFromPubkeysBtc(pubkeys: Uint8Array[], m: number) {
          return b58ToC32(p2sh(p2ms(m, pubkeys)).address!, 0x15);
        }
        for (let n = 1; n <= 15; n++) {
          const pubkeys = randomPublicKeys(n);
          for (let m = 1; m <= 15; m++) {
            if (m > n) continue;
            const scurePrincipal = principalFromPubkeysBtc(pubkeys, m);
            const contractPrincipal = rov(
              signers.pubkeysToPrincipal(pubkeys, m)
            );
            expect(contractPrincipal).toEqual(scurePrincipal);
          }
        }
      });
    });
  });
});
