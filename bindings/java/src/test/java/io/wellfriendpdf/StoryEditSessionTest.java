package io.wellfriendpdf;

import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.HexFormat;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;
import static org.junit.Assert.*;

/** Unexecuted binding regression source; does not qualify rendering or editing. */
public final class StoryEditSessionTest {
    private static byte[] fixture() throws Exception {
        for (Path dir = Path.of("").toAbsolutePath(); dir != null; dir = dir.getParent()) {
            Path file = dir.resolve("crates/engine/tests/fixtures/basicapi.pdf");
            if (Files.isRegularFile(file)) return Files.readAllBytes(file);
        }
        throw new IllegalStateException("basicapi.pdf fixture not found");
    }
    @Test public void ownsInputCancellationAndThreadConfinement() throws Exception {
        byte[] bytes = fixture();
        byte[] original = bytes.clone();
        WellfriendPdf.StoryEditSession session = new WellfriendPdf.StoryEditSession(bytes);
        try {
            Arrays.fill(bytes, (byte) 0);
            assertArrayEquals(original, session.bytes());
            assertTrue(session.statusJson().contains(HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(original))));
            assertFalse(session.undo());
            assertFalse(session.redo());
            assertThrows(WellfriendPdf.WellfriendPdfException.class,
                () -> session.commandJson("{\"op\":\"checkpoint\"}"));
            assertThrows(IllegalArgumentException.class,
                () -> session.commandJson("{\"op\":\"status\",\"bad\":\"\uD800\"}"));
            try (WellfriendPdf.RenderCancellation cancellation = new WellfriendPdf.RenderCancellation()) {
                Thread signal = new Thread(cancellation::cancel);
                signal.start(); signal.join();
                assertThrows(WellfriendPdf.WellfriendPdfException.class,
                    () -> session.commandJson("{\"op\":\"undo\"}", cancellation));
            }
            AtomicReference<Throwable> rejected = new AtomicReference<>();
            Thread other = new Thread(() -> {
                try { session.statusJson(); } catch (Throwable ex) { rejected.set(ex); }
            });
            other.start(); other.join();
            assertTrue(rejected.get() instanceof IllegalStateException);
            assertArrayEquals(original, session.bytes());
        } finally { session.close(); }
        session.close();
        assertThrows(IllegalStateException.class, session::statusJson);
    }

    @Test public void passwordEntrypointsDoNotRetainCredentialsForPlainInput() throws Exception {
        byte[] bytes = fixture();
        try (WellfriendPdf.StoryEditSession text =
                 new WellfriendPdf.StoryEditSession(bytes, "ignored-for-unencrypted");
             WellfriendPdf.StoryEditSession exact =
                 WellfriendPdf.StoryEditSession.openWithPasswordBytes(
                     bytes, new byte[] {(byte) 0xff, 0, 0x61})) {
            for (String status : new String[] {text.statusJson(), exact.statusJson()}) {
                assertTrue(status.contains("\"source_was_encrypted\":false"));
                assertTrue(status.contains("\"working_copy_decrypted\":false"));
                assertTrue(status.contains("\"password_retained\":false"));
            }
        }
    }
}
