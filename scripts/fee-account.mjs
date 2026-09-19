#!/usr/bin/env node
// Wrapped-SOL fee account helper.
//
// The account that receives platform fees only ever *receives*, so it needs no
// signature to operate and its owner can be a hardware wallet that stays
// unplugged. This script derives that account's address, checks it on chain,
// and creates it if asked.
//
//   node scripts/fee-account.mjs derive <owner-address>
//   node scripts/fee-account.mjs check  <token-account-or-owner-address>
//   node scripts/fee-account.mjs create <owner-address> <payer-secret-base58>
//
// `derive` and `check` touch no funds and need no keys.
import {
  Connection, PublicKey, Keypair, Transaction, TransactionInstruction, SystemProgram,
  sendAndConfirmTransaction,
} from '@solana/web3.js';

const WSOL = new PublicKey('So11111111111111111111111111111111111111112');
const TOKEN_PROGRAM = new PublicKey('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const ATA_PROGRAM = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
const RPC = process.env.SOLANA_RPC_URL || 'https://api.mainnet-beta.solana.com';

/** The associated token account for an owner and mint. Deterministic: the same
 *  owner always yields the same address, whether or not it exists yet. */
function associatedAddress(owner, mint = WSOL) {
  return PublicKey.findProgramAddressSync(
    [owner.toBuffer(), TOKEN_PROGRAM.toBuffer(), mint.toBuffer()],
    ATA_PROGRAM,
  )[0];
}

function owner(arg) {
  try { return new PublicKey(arg); }
  catch { fail(`"${arg}" is not a valid Solana address.`); }
}
function fail(message) { console.error(`\n  ${message}\n`); process.exit(1); }

async function check(connection, address) {
  const info = await connection.getParsedAccountInfo(address);
  if (!info.value) return { exists: false };
  const parsed = info.value.data?.parsed;
  return {
    exists: true,
    isTokenAccount: parsed?.type === 'account',
    mint: parsed?.info?.mint,
    owner: parsed?.info?.owner,
    // A wrapped-SOL account holds rent plus whatever fees have accrued.
    lamports: info.value.lamports,
    amount: parsed?.info?.tokenAmount?.uiAmount,
  };
}

const [command, ...args] = process.argv.slice(2);
const connection = new Connection(RPC, 'confirmed');

if (command === 'derive' || command === 'create') {
  const ownerKey = owner(args[0]);
  const ata = associatedAddress(ownerKey);
  console.log(`\n  Owner (your Ledger)   ${ownerKey.toBase58()}`);
  console.log(`  WSOL fee account      ${ata.toBase58()}`);
  console.log(`\n  Put the fee account — not the owner — in PLATFORM_FEE_ACCOUNT.`);

  const state = await check(connection, ata);
  console.log(`  On chain              ${state.exists ? 'exists' : 'does not exist yet'}`);

  if (command === 'derive') {
    if (!state.exists) console.log(`\n  To create it:  node scripts/fee-account.mjs create ${ownerKey.toBase58()} <payer-secret-base58>\n`);
    else console.log('');
    process.exit(0);
  }
  if (state.exists) fail('It already exists. Nothing to do.');

  const secret = args[1];
  if (!secret) fail('Creating it needs a funded payer: pass a base58 secret key as the last argument.');
  let payer;
  try {
    payer = Keypair.fromSecretKey(Uint8Array.from(Buffer.from(
      (await import('bs58')).default.decode(secret))));
  } catch { fail('The payer secret key could not be read as base58.'); }

  const balance = await connection.getBalance(payer.publicKey);
  console.log(`  Payer                 ${payer.publicKey.toBase58()} (${balance / 1e9} SOL)`);
  if (balance < 3_000_000) fail('The payer needs about 0.003 SOL to cover rent and the network fee.');

  // ATA program, "create idempotent" (instruction 1). The payer funds the rent;
  // the owner neither signs nor needs to be present.
  const instruction = new TransactionInstruction({
    programId: ATA_PROGRAM,
    keys: [
      { pubkey: payer.publicKey, isSigner: true, isWritable: true },
      { pubkey: ata, isSigner: false, isWritable: true },
      { pubkey: ownerKey, isSigner: false, isWritable: false },
      { pubkey: WSOL, isSigner: false, isWritable: false },
      { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
      { pubkey: TOKEN_PROGRAM, isSigner: false, isWritable: false },
    ],
    data: Buffer.from([1]),
  });
  const signature = await sendAndConfirmTransaction(
    connection, new Transaction().add(instruction), [payer]);
  console.log(`\n  Created. Signature    ${signature}`);
  console.log(`  Set PLATFORM_FEE_ACCOUNT=${ata.toBase58()}\n`);
} else if (command === 'check') {
  const given = owner(args[0]);
  // Accept either the token account or its owner, since it is easy to confuse.
  for (const [label, address] of [['as given', given], ['derived from it as owner', associatedAddress(given)]]) {
    const state = await check(connection, address);
    console.log(`\n  ${address.toBase58()}  (${label})`);
    if (!state.exists) { console.log('    does not exist'); continue; }
    console.log(`    token account       ${state.isTokenAccount ? 'yes' : 'no — this cannot receive fees'}`);
    if (state.mint) console.log(`    mint                ${state.mint}${state.mint === WSOL.toBase58() ? '  (wrapped SOL)' : '  — NOT wrapped SOL'}`);
    if (state.owner) console.log(`    owned by            ${state.owner}`);
    if (state.amount != null) console.log(`    fees accrued        ${state.amount} WSOL`);
  }
  console.log('');
} else {
  console.log(`
  Wrapped-SOL fee account helper

    derive <owner-address>                     show the fee account address
    check  <address>                           inspect it on chain
    create <owner-address> <payer-secret>      create it (payer pays ~0.003 SOL)

  derive and check touch no funds and need no keys.
`);
}
