import { test } from "node:test";
import { verifyResults } from "../../scripts/openshell/verify-results.mjs";
test("free OpenShell round trip exports an exact patch and preserves the source", { skip: process.env.XENONET_REVIEW_REAL !== "1", timeout: 300000 }, async () => { await verifyResults(); });
