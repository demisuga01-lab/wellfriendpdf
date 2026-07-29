import io.wellfriendpdf.Wellfriend;

public final class ReleasePackagingCodecIsolation {
    public static void main(String[] args) {
        byte[] encoded = new byte[] {
            (byte) 0x78, (byte) 0x9c, (byte) 0xcb, (byte) 0x48,
            (byte) 0xcd, (byte) 0xc9, (byte) 0xc9, (byte) 0x57,
            (byte) 0xc8, (byte) 0xaf, (byte) 0xc8, (byte) 0x4c,
            (byte) 0x49, (byte) 0x05, (byte) 0x00, (byte) 0x19,
            (byte) 0xdd, (byte) 0x04, (byte) 0x4e
        };
        String json = WellfriendPdf.codecIsolationReportJson("FlateDecode", encoded, "in_process");
        System.out.println(json);
        if (!json.contains("\"status\":\"success\"")
                && !json.contains("\"status\": \"success\"")) {
            throw new IllegalStateException("codec isolation report was not successful");
        }
    }
}
