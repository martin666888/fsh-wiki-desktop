import assert from "node:assert/strict";
import { verifyInstaller } from "./verify-signature.mjs";
import { compareStableVersions, parseVersion } from "./check-project.mjs";

// Only public data: no production key, private key or runnable installer.
// The Ed/ED compatibility vectors are from minisign-verify 0.2.5 (MIT).
const upstreamKey = Buffer.from("untrusted comment: fixture\nRWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3").toString("base64");
const vectors = [
  ["RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=", "timestamp:1555779966\tfile:test", "QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA=="],
  ["RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=", "timestamp:1556193335\tfile:test", "y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg=="],
];
for (const [signature, comment, global] of vectors) {
  const text = Buffer.from(`untrusted comment: fixture\n${signature}\ntrusted comment: ${comment}\n${global}`).toString("base64");
  // Both signatures must pass crypto verification before the missing-version guard rejects them.
  assert.throws(() => verifyInstaller(Buffer.from("test"), text, upstreamKey, "2.0.0"), /签名必须且只能包含一个可信版本号/);
  assert.throws(() => verifyInstaller(Buffer.from("Test"), text, upstreamKey, "2.0.0"), /安装器签名验证失败/);
}

// Same non-executable public fixture checked independently by Rust's minisign verifier.
const key = "dW50cnVzdGVkIGNvbW1lbnQ6IGdlbmVyYXRlZCB0ZXN0IGZpeHR1cmUKUldRQkFnTUVCUVlIQ0hYMENzZG5nUzBWSG05T0dya2NURHZyUjdjU2t2MnN0WkJwOEFtSlozRSs=";
const signature = "dW50cnVzdGVkIGNvbW1lbnQ6IGdlbmVyYXRlZCB0ZXN0IGZpeHR1cmUKUldRQkFnTUVCUVlIQ1AxOE96bG5LT25ZRXgvM3BoNU1xK2JYa2Zqd1lCQjQ2UjlPQ0ZHNXQwV0RmYkx5am9SVDBBcm5qMjE3WEcxYTRmNjBlSHlMdFRpaCtNZGZEUTh1QVEwPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxCWZpbGU6Zml4dHVyZS5leGUJdmVyc2lvbjoyLjAuMApIZmJkMDJCd1dacGlUZjVlOTZIb0l0clB1TjBkWDhJNFV0R2pjT1U1QkY4Z2pXbWJmWTR2bG1qS2xocno5V2M1M3dwaHpVVnFsOHFENW5RYnVZZTVBdz09";
const bytes = Buffer.concat([Buffer.from("MZ"), Buffer.alloc(18), Buffer.from("efbeadde4e756c6c736f6674496e7374", "hex")]);
verifyInstaller(bytes, signature, key, "2.0.0");
assert.throws(() => verifyInstaller(Buffer.from("tampered"), signature, key, "2.0.0"), /安装器签名验证失败/);
assert.throws(() => verifyInstaller(bytes, signature, key, "2.0.1"), /签名版本与发布版本不一致/);
const changedComment = Buffer.from(Buffer.from(signature, "base64").toString("utf8").replace("version:2.0.0", "version:2.0.1")).toString("base64");
assert.throws(() => verifyInstaller(bytes, changedComment, key, "2.0.1"), /可信注释签名验证失败/);
assert.throws(() => verifyInstaller(bytes, signature, upstreamKey, "2.0.0"), /key id 不一致/);
assert.equal(parseVersion("1.2.3-beta.1").prerelease, true);
assert.throws(() => parseVersion("1.2.3-beta.01"));
assert.equal(compareStableVersions("1.10.0", "1.9.9"), 1);
assert.equal(compareStableVersions("1.2.3+new", "1.2.3+old"), 0);
console.log("公开签名向量、篡改拒绝、可信版本绑定与稳定版顺序检查通过");
