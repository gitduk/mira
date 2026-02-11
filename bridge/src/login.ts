/**
 * WhatsApp Authentication Script for Mira
 *
 * Run this to authenticate with WhatsApp.
 * Displays QR code, waits for scan, saves credentials, then exits.
 */
import {
  makeWASocket,
  DisconnectReason,
  useMultiFileAuthState,
  makeCacheableSignalKeyStore,
  fetchLatestBaileysVersion,
} from "@whiskeysockets/baileys";
import path from "path";
import fs from "fs";
import pino from "pino";
import qrcode from "qrcode-terminal";

// Default to ../store/whatsapp if not provided, assuming we run from mira/bridge
const STORE_DIR =
  process.env.STORE_DIR || path.resolve(process.cwd(), "../store/whatsapp");

// Minimal logging to avoid noisy Baileys logs
const logger = pino({ level: "silent" });

async function authenticate() {
  const authDir = path.join(STORE_DIR, "auth");
  console.log(`Using store directory: ${STORE_DIR}`);

  // Create auth directory if it doesn't exist
  fs.mkdirSync(authDir, { recursive: true });

  const { state, saveCreds } = await useMultiFileAuthState(authDir);

  // Check if already authenticated
  if (state.creds.me && state.creds.me.id) {
    console.log("\n✓ Already authenticated with WhatsApp");
    console.log(
      `  User: ${state.creds.me.name || state.creds.me.id.split(":")[0]}`,
    );
    console.log(
      "  To re-authenticate, delete the store/auth folder and run again:",
    );
    console.log(`  rm -rf ${authDir}`);
    process.exit(0);
  }

  console.log("Starting WhatsApp authentication...");

  // Always use the latest WA Web version Baileys supports to avoid 405 errors
  const { version } = await fetchLatestBaileysVersion();

  const maxRetries = 5;
  let retryCount = 0;
  let retryTimer: NodeJS.Timeout | null = null;

  const startSocket = () => {
    const sock = makeWASocket({
      auth: {
        creds: state.creds,
        keys: makeCacheableSignalKeyStore(state.keys, logger as any),
      },
      printQRInTerminal: false,
      logger: logger as any,
      version,
      browser: ["Mira", "Chrome", "1.0.0"],
      shouldSyncHistoryMessage: () => false,
    });

    sock.ev.on("creds.update", saveCreds);

    sock.ev.on("connection.update", (update) => {
      const { connection, lastDisconnect, qr } = update;

      if (qr) {
        console.log("\nScan this QR code with WhatsApp:");
        console.log("  1. Open WhatsApp on your phone");
        console.log("  2. Tap Settings → Linked Devices → Link a Device");
        console.log("  3. Point your camera at the QR code below\n");
        qrcode.generate(qr, { small: true });
      }

      if (connection === "close") {
        const reason = (lastDisconnect?.error as any)?.output?.statusCode;
        if (reason === DisconnectReason.loggedOut) {
          console.log(
            "\n✗ Logged out. Delete store/whatsapp/auth and try again.",
          );
          process.exit(1);
        } else {
          if (retryCount >= maxRetries) {
            console.log("✗ Max retries reached. Exiting.");
            process.exit(1);
          }
          retryCount += 1;
          if (retryTimer) clearTimeout(retryTimer);
          retryTimer = setTimeout(() => {
            startSocket();
          }, 4000);
        }
      }

      if (connection === "open") {
        console.log("\n✓ Successfully authenticated with WhatsApp!");
        console.log("  Credentials saved to store/whatsapp/auth/");
        console.log("  You can now start the Mira service.\n");

        // Give it a moment to save credentials, then exit
        setTimeout(() => process.exit(0), 1000);
      }
    });
  };

  startSocket();
}

authenticate().catch((err) => {
  console.error("Authentication failed:", err);
  process.exit(1);
});
