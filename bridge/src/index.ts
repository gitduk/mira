/**
 * Mira WhatsApp Bridge
 * Lightweight Node.js process that handles WhatsApp protocol via baileys.
 * Communicates with Rust main process via stdin/stdout JSON Lines.
 */

import fs from "fs";
import path from "path";
import readline from "readline";
import {
  makeWASocket,
  DisconnectReason,
  WASocket,
  jidNormalizedUser,
  makeCacheableSignalKeyStore,
  useMultiFileAuthState,
  fetchLatestBaileysVersion,
} from "@whiskeysockets/baileys";

const STORE_DIR = process.env.STORE_DIR || "../store/whatsapp";

// JSON Lines output to stdout
function emit(event: object): void {
  process.stdout.write(JSON.stringify(event) + "\n");
}

// Silence baileys internal logging
const logger = {
  level: "silent" as const,
  trace: () => {},
  debug: () => {},
  info: () => {},
  warn: () => {},
  error: (...args: any[]) => {
    process.stderr.write(`[baileys] ${args.join(" ")}\n`);
  },
  fatal: (...args: any[]) => {
    process.stderr.write(`[baileys FATAL] ${args.join(" ")}\n`);
  },
  child: () => logger,
};

let sock: WASocket;
let lidToPhoneMap: Record<string, string> = {};

async function translateJid(jid: string): Promise<string> {
  if (!jid.endsWith("@lid")) return jid;
  const lidUser = jid.split("@")[0].split(":")[0];

  const cached = lidToPhoneMap[lidUser];
  if (cached) return cached;

  try {
    const pnJid = await sock.signalRepository.lidMapping.getPNForLID(jid);
    if (pnJid) {
      const normalized = jidNormalizedUser(pnJid);
      if (normalized) {
        lidToPhoneMap[lidUser] = normalized;
        return normalized;
      }
    }
  } catch {}

  return jid;
}

async function connect(): Promise<void> {
  const authDir = path.join(STORE_DIR, "auth");
  fs.mkdirSync(authDir, { recursive: true });

  const { state, saveCreds } = await useMultiFileAuthState(authDir);
  const { version } = await fetchLatestBaileysVersion();

  sock = makeWASocket({
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

  sock.ev.on("connection.update", (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      emit({ type: "qr", code: qr });
      return;
    }

    if (connection === "close") {
      const reason = (lastDisconnect?.error as any)?.output?.statusCode;
      emit({ type: "connection", status: "close", reason });

      const shouldReconnect =
        reason !== DisconnectReason.loggedOut &&
        reason !== DisconnectReason.connectionReplaced;

      if (shouldReconnect) {
        setTimeout(() => connect(), 5000);
      } else {
        process.exit(0);
      }
    } else if (connection === "open") {
      emit({ type: "connection", status: "open" });

      // Build LID mapping
      if (sock.user) {
        const phoneUser = sock.user.id.split(":")[0];
        const lidUser = sock.user.lid?.split(":")[0];
        if (lidUser && phoneUser) {
          lidToPhoneMap[lidUser] = `${phoneUser}@s.whatsapp.net`;
        }
      }

      emit({
        type: "ready",
        user_jid: sock.user?.id,
        lid_jid: sock.user?.lid,
      });
    }
  });

  sock.ev.on("creds.update", saveCreds);

  sock.ev.on("messages.upsert", ({ messages }) => {
    (async () => {
      for (const msg of messages) {
        if (!msg.message) continue;
        const rawJid = msg.key.remoteJid;
        if (!rawJid || rawJid === "status@broadcast") continue;

        const chatJid = await translateJid(rawJid);
        const timestamp = new Date(
          Number(msg.messageTimestamp) * 1000,
        ).toISOString();
        const content =
          msg.message?.conversation ||
          msg.message?.extendedTextMessage?.text ||
          msg.message?.imageMessage?.caption ||
          msg.message?.videoMessage?.caption ||
          "";

        const sender = msg.key.participant || msg.key.remoteJid || "";
        const senderName = msg.pushName || sender.split("@")[0];

        emit({
          type: "message",
          msg_id: msg.key.id || "",
          chat_jid: chatJid,
          sender,
          sender_name: senderName,
          content,
          timestamp,
          is_from_me: msg.key.fromMe || false,
        });

        // Also emit chat metadata
        emit({
          type: "chat_metadata",
          chat_jid: chatJid,
          timestamp,
        });
      }
    })().catch((err) => {
      process.stderr.write(`[bridge] Message handler error: ${err}\n`);
    });
  });
}

// Read commands from stdin (JSON Lines)
const rl = readline.createInterface({ input: process.stdin });
rl.on("line", async (line: string) => {
  try {
    const cmd = JSON.parse(line.trim());
    switch (cmd.type) {
      case "send_message":
        if (sock && cmd.jid && cmd.text) {
          const result = await sock.sendMessage(cmd.jid, { text: cmd.text });
          emit({
            type: "message_sent",
            msg_id: result?.key?.id || "",
            jid: cmd.jid,
          });
        }
        break;
      case "fetch_groups":
        if (sock) {
          const groups = await sock.groupFetchAllParticipating();
          emit({
            type: "groups_result",
            groups: Object.entries(groups).map(([jid, meta]) => ({
              jid,
              subject: meta.subject || "",
            })),
          });
        }
        break;
      case "shutdown":
        process.exit(0);
        break;
    }
  } catch (err) {
    emit({ type: "error", message: `Command parse error: ${err}` });
  }
});

// Start
connect().catch((err) => {
  emit({ type: "error", message: `Connection failed: ${err}` });
  process.exit(1);
});
