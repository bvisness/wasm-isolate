// "Pseudocode" for the RLE encoding used in this project. Working out the logic
// in JS makes it easier to transcribe the logic into handwritten WASM.

// This should actually be 255 in practice
const max = 4;

function encode(mem) {
  const out = [];

  let i = 0;
  let mode = 0;
  let current_byte = '?';
  let current_count = 0;
  while (i < mem.length) {
    current_byte = mem[i];
    current_count = 1;
    i++;

    while (i < mem.length && current_count < max) {
      // This whole thing can conveniently be handled as an xor
      if (
        (/* run mode */ mode === 0 && /* run ended */ mem[i] !== current_byte)
        || (/* literal mode */ mode !== 0 && /* literals ended */ mem[i] === current_byte)
      ) {
        break;
      }

      current_count++;
      current_byte = mem[i];
      i++;
    }

    if (mode === 0) {
      // flush run
      out.push(current_count);
      out.push(current_byte);
    } else {
      // flush literals
      out.push(current_count);
      out.push(...mem.slice(i - current_count, i)); // or: start=i-currentCount, len=currentCount
    }
    mode = (mode + 1) % 2; // toggle
  }

  return out;
}

function decode(buf) {
  // The pseudocode here is a bit weird because we are just pushing to the
  // the output instead of writing into it, so we don't need that separate `i`
  // variable that we have in the real code.
  const out = [];

  let cur = 0;
  let mode = 0;
  while (cur < buf.length) {
    let n = buf[cur];
    cur++;

    if (mode === 0) {
      // run
      out.push(...new Array(n).fill(buf[cur])); // memory.fill where start=cur, len=n
      cur++;
    } else {
      // literals
      out.push(...buf.slice(cur, cur + n)); // memory.copy where src=cur, dst=i, len=n
      cur += n;
    }

    mode = (mode + 1) % 2;
  }

  return out;
}

function test(original, expected) {
  function compareArrays(mode, expected, actual) {
    console.log(`(${mode}) Expected:`, JSON.stringify(expected));
    console.log(`(${mode})   Actual:`, JSON.stringify(actual));
    if (actual.length !== expected.length) {
      throw new Error(`Expected output of length ${expected.length}, but got length ${actual.length}`);
    }
    for (let i = 0; i < actual.length; i++) {
      if (actual[i] !== expected[i]) {
        throw new Error(`At index ${i}: expected ${expected[i]} but got ${actual[i]}`);
      }
    }
  }

  console.log("    Original:", JSON.stringify(original));
  compareArrays("E", expected, encode(original));
  compareArrays("D", original, decode(encode(original)));
}

test(
  [],
  [],
);
test(
  ['A', 'A', 'A', 'A'],
  [4, 'A'],
);
test(
  ['A', 'A', 'A', 'A', 'B'],
  [4, 'A', 1, 'B'],
);
test(
  ['A', 'A', 'A', 'A', 'B', 'C', 'D', 'D', 'A', 'B', 'C', 'D'],
  [4, 'A', 3, 'B', 'C', 'D', 1, 'D', 4, 'A', 'B', 'C', 'D'],
);
test(
  ['A', 'B', 'C', 'D'],
  [1, 'A', 3, 'B', 'C', 'D'],
);
test(
  ['A', 'B', 'C', 'D'],
  [1, 'A', 3, 'B', 'C', 'D'],
);
test(
  ['A', 'A', 'A', 'B', 'B', 'B', 'C', 'C', 'C'],
  [3, 'A', 1, 'B', 2, 'B', 1, 'C', 2, 'C'],
);
test(
  ['A', 'A', 'A', 'B'],
  [3, 'A', 1, 'B'],
);
test(
  ['A', 'A', 'A', 'A', 'A', 'A', 'A', 'A'],
  [4, 'A', 1, 'A', 3, 'A'],
);
test(
  ['A', 'A', 'A', 'A', 'A', 'B', 'B', 'B', 'B', 'B', 'B'],
  [4, 'A', 2, 'A', 'B', 4, 'B', 1, 'B'],
);
