#!/usr/bin/env node
// Deterministic fixture: it never loads an SDK, provider or credential.
import fs from "node:fs";
fs.writeFileSync("hello.txt", "Synthetic sandbox result\r\n");
fs.writeFileSync("added.txt", "Added without a final newline");
fs.writeFileSync("empty.txt", "");
fs.unlinkSync("removed.txt");
fs.writeFileSync("ignored.txt", "This generated file must stay outside the patch.");
