import java.util.*;
import javax.crypto.*;
import javax.crypto.spec.*;

public final class PrimitiveInterop {
  static byte[] hex(String s) {
    s = s.replaceAll("\\s", "");
    byte[] out = new byte[s.length() / 2];
    for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(s.substring(2*i, 2*i+2), 16);
    return out;
  }
  static String hex(byte[] b) {
    StringBuilder sb = new StringBuilder();
    for (byte x : b) sb.append(String.format("%02x", x & 0xff));
    return sb.toString();
  }
  static void requireEq(String name, byte[] actual, String expected) {
    if (!hex(actual).equals(expected.toLowerCase(Locale.ROOT))) throw new RuntimeException(name + " mismatch: " + hex(actual));
  }
  static byte[] hmac(byte[] key, byte[] msg) throws Exception {
    Mac mac = Mac.getInstance("HmacSHA256");
    mac.init(new SecretKeySpec(key, "HmacSHA256"));
    return mac.doFinal(msg);
  }
  static byte[] hkdf(byte[] ikm, byte[] salt, byte[] info, int len) throws Exception {
    byte[] prk = hmac(salt, ikm);
    byte[] okm = new byte[len];
    byte[] prev = new byte[0];
    int pos = 0;
    int counter = 1;
    while (pos < len) {
      Mac mac = Mac.getInstance("HmacSHA256");
      mac.init(new SecretKeySpec(prk, "HmacSHA256"));
      mac.update(prev);
      mac.update(info);
      mac.update((byte) counter++);
      prev = mac.doFinal();
      int take = Math.min(prev.length, len - pos);
      System.arraycopy(prev, 0, okm, pos, take);
      pos += take;
    }
    Arrays.fill(prk, (byte) 0);
    Arrays.fill(prev, (byte) 0);
    return okm;
  }
  public static void main(String[] args) throws Exception {
    Cipher gcm = Cipher.getInstance("AES/GCM/NoPadding");
    byte[] gcmKey = hex("00000000000000000000000000000000");
    byte[] gcmIv = hex("000000000000000000000000");
    byte[] gcmPlain = hex("00000000000000000000000000000000");
    gcm.init(Cipher.ENCRYPT_MODE, new SecretKeySpec(gcmKey, "AES"), new GCMParameterSpec(128, gcmIv));
    requireEq("AES-GCM", gcm.doFinal(gcmPlain), "0388dace60b6a392f328c2b971b2fe78ab6e47d42cec13bdf53a67b21257bddf");

    Cipher kw = Cipher.getInstance("AESWrap");
    byte[] kek = hex("000102030405060708090a0b0c0d0e0f");
    byte[] keyData = hex("00112233445566778899aabbccddeeff");
    kw.init(Cipher.WRAP_MODE, new SecretKeySpec(kek, "AES"));
    byte[] wrapped = kw.wrap(new SecretKeySpec(keyData, "AES"));
    requireEq("AES-KW", wrapped, "1fa68b0a8112b447aef34bd8fb5a7b829d3e862371d2cfe5");

    requireEq("HMAC-SHA256", hmac(hex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b"), "Hi There".getBytes("US-ASCII")), "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");

    byte[] ikm = new byte[22]; Arrays.fill(ikm, (byte) 0x0b);
    requireEq("HKDF-SHA256", hkdf(ikm, hex("000102030405060708090a0b0c"), hex("f0f1f2f3f4f5f6f7f8f9"), 42), "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865");

    System.out.println("{\"status\":\"passed\",\"aes_gcm\":\"match\",\"aes_kw\":\"match\",\"hmac_sha256\":\"match\",\"hkdf_sha256\":\"match\"}");
  }
}
