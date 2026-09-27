import assert from "node:assert/strict";
import { createHash, createPublicKey, verify } from "node:crypto";

function decodeBase64(value, name) {
  assert.equal(typeof value, "string", `${name} 必须是字符串`);
  const text = value.trim();
  assert.ok(text.length > 0 && /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(text), `${name} 不是规范 base64`);
  return Buffer.from(text, "base64");
}

// Tauri 的公钥和 .sig 都是 base64 包裹的 minisign 文本。
// 格式依据 minisign-verify 0.2.5；密码运算使用 Node/OpenSSL，未自行实现算法。
// 同时验证数据签名和可信注释签名，之后才信任 version 字段。
export function verifyInstaller(bytes, signatureText, publicKeyText, expectedVersion) {
  const publicLines = decodeBase64(publicKeyText, "公钥").toString("utf8").trim().split(/\r?\n/);
  assert.equal(publicLines.length, 2, "公钥文本结构无效");
  const pub = decodeBase64(publicLines[1], "公钥主体");
  assert.equal(pub.length, 42, "公钥长度无效");
  assert.ok(["Ed", "ED"].includes(pub.subarray(0, 2).toString("ascii")), "不支持的公钥算法");
  const lines = decodeBase64(signatureText, "签名").toString("utf8").trim().split(/\r?\n/);
  assert.equal(lines.length, 4, "签名文本结构无效");
  assert.ok(lines[2].startsWith("trusted comment: "), "缺少可信注释");
  const packet = decodeBase64(lines[1], "数据签名");
  const globalSignature = decodeBase64(lines[3], "注释签名");
  assert.equal(packet.length, 74, "签名长度无效");
  assert.equal(globalSignature.length, 64, "注释签名长度无效");
  assert.ok(packet.subarray(2, 10).equals(pub.subarray(2, 10)), "签名与公钥的 key id 不一致");
  const algorithm = packet.subarray(0, 2).toString("ascii");
  assert.ok(["Ed", "ED"].includes(algorithm), "不支持的签名算法");
  const key = createPublicKey({
    key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), pub.subarray(10)]),
    format: "der",
    type: "spki",
  });
  const signature = packet.subarray(10);
  const message = algorithm === "ED" ? createHash("blake2b512").update(bytes).digest() : bytes;
  assert.ok(verify(null, message, key, signature), "安装器签名验证失败");
  const comment = lines[2].slice("trusted comment: ".length);
  assert.ok(verify(null, Buffer.concat([signature, Buffer.from(comment, "utf8")]), key, globalSignature), "可信注释签名验证失败");
  const versions = comment.split("\t").filter((field) => field.startsWith("version:"));
  assert.equal(versions.length, 1, "签名必须且只能包含一个可信版本号");
  assert.equal(versions[0].slice("version:".length), expectedVersion, "签名版本与发布版本不一致");
}
