#!/usr/bin/env node
// A fake `avenger quarto2-engine` for the extension's tests: it writes the request it reads to
// FAKE_REQUEST, a file, and answers FAKE_RESPONSE, a JSON text, or the text of the file
// FAKE_RESPONSE_FILE, with the exit status FAKE_STATUS.

import { readFileSync, writeFileSync } from "node:fs";

let input = "";
process.stdin.on("data", (chunk) => (input += chunk));
process.stdin.on("end", () => {
  if (process.env.FAKE_REQUEST) {
    writeFileSync(process.env.FAKE_REQUEST, input);
  }
  const response = process.env.FAKE_RESPONSE_FILE
    ? readFileSync(process.env.FAKE_RESPONSE_FILE, "utf8")
    : (process.env.FAKE_RESPONSE ?? "");
  // Exit once the whole response is written, which can take several writes of the pipe.
  process.stdout.write(response, () => process.exit(Number(process.env.FAKE_STATUS ?? 0)));
});
