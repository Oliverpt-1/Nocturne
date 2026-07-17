// Baseline: how a maker signs Midnight offer trees today - ethers v6, single-threaded.
// Computes the SAME leaves/tree as the Rust crate, so its N=4 root must equal the Rust CROSSCHECK_ROOT.
const { TypedDataEncoder, keccak256, concat, toBeHex, zeroPadValue, SigningKey } = require("ethers");

const A = (b) => "0x" + b.toString(16).padStart(2, "0").repeat(20);
const RATIFIER = A(0xbb);

const types = {
  CollateralParams: [
    { name: "token", type: "address" },
    { name: "lltv", type: "uint256" },
    { name: "liquidationCursor", type: "uint256" },
    { name: "oracle", type: "address" },
  ],
  Market: [
    { name: "chainId", type: "uint256" },
    { name: "midnight", type: "address" },
    { name: "loanToken", type: "address" },
    { name: "collateralParams", type: "CollateralParams[]" },
    { name: "maturity", type: "uint256" },
    { name: "rcfThreshold", type: "uint256" },
    { name: "enterGate", type: "address" },
    { name: "liquidatorGate", type: "address" },
  ],
  Offer: [
    { name: "market", type: "Market" },
    { name: "buy", type: "bool" },
    { name: "maker", type: "address" },
    { name: "start", type: "uint256" },
    { name: "expiry", type: "uint256" },
    { name: "tick", type: "uint256" },
    { name: "group", type: "bytes32" },
    { name: "callback", type: "address" },
    { name: "callbackData", type: "bytes" },
    { name: "receiverIfMakerIsSeller", type: "address" },
    { name: "ratifier", type: "address" },
    { name: "reduceOnly", type: "bool" },
    { name: "maxUnits", type: "uint128" },
    { name: "maxAssets", type: "uint128" },
    { name: "continuousFeeCap", type: "uint256" },
  ],
};

const ZERO = "0x0000000000000000000000000000000000000000";

function sampleOffer(i) {
  return {
    market: {
      chainId: 1,
      midnight: A(0x11),
      loanToken: A(0x22),
      collateralParams: [
        { token: A(0x33), lltv: 860000000000000000n, liquidationCursor: 1, oracle: A(0x44) },
      ],
      maturity: 1800000000,
      rcfThreshold: 1000,
      enterGate: ZERO,
      liquidatorGate: ZERO,
    },
    buy: i % 2 === 0,
    maker: A(0x55),
    start: 0,
    expiry: 2000000000,
    tick: i % 6744,
    group: zeroPadValue(toBeHex(i), 32),
    callback: ZERO,
    callbackData: "0x",
    receiverIfMakerIsSeller: ZERO,
    ratifier: RATIFIER,
    reduceOnly: false,
    maxUnits: 1000000 + i,
    maxAssets: 0,
    continuousFeeCap: 0,
  };
}

const hashNode = (l, r) => keccak256(concat([l, r]));

function buildTree(leaves) {
  const levels = [leaves];
  while (levels[levels.length - 1].length > 1) {
    const prev = levels[levels.length - 1];
    const next = [];
    for (let i = 0; i < prev.length; i += 2) next.push(hashNode(prev[i], prev[i + 1]));
    levels.push(next);
  }
  return levels;
}
function proof(levels, index) {
  const h = levels.length - 1;
  const p = [];
  let idx = index;
  for (let l = 0; l < h; l++) {
    p.push(levels[l][idx ^ 1]);
    idx >>= 1;
  }
  return p;
}

const sk = new SigningKey("0x" + "42".repeat(32));

function pipeline(offers) {
  const t = process.hrtime.bigint();
  const leaves = offers.map((o) => TypedDataEncoder.hashStruct("Offer", types, o));
  const levels = buildTree(leaves);
  for (let i = 0; i < offers.length; i++) proof(levels, i); // takers need proofs
  const root = levels[levels.length - 1][0];
  // one signature covers the whole tree
  const structHash = keccak256(concat([keccak256(Buffer.from("dummy")), root])); // typehash cost is O(1); shape-equivalent
  sk.sign(structHash);
  return { root, us: Number((process.hrtime.bigint() - t) / 1000n) };
}

// cross-check vs Rust
const small = [0, 1, 2, 3].map(sampleOffer);
console.log("CROSSCHECK_ROOT_N4", pipeline(small).root);

for (const n of [1024, 4096, 16384]) {
  const offers = Array.from({ length: n }, (_, i) => sampleOffer(i));
  pipeline(offers); // warmup
  let best = Infinity;
  for (let r = 0; r < 20; r++) best = Math.min(best, pipeline(offers).us);
  console.log(`RESULT n=${n} single_us=${best} single_per_offer_ns=${((best * 1000) / n).toFixed(0)}`);
}
