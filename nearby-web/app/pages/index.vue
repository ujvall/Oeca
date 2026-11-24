<!-- Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md. -->
<template>
  <div class="mx-auto flex min-h-dvh w-full max-w-[28rem] flex-col px-4 pt-[max(0.5rem,env(safe-area-inset-top))] pb-[max(0.75rem,env(safe-area-inset-bottom))] sm:pt-8">
    <!-- The notch: always what Nearby is doing. Settings at the side -->
    <header class="relative flex h-14 shrink-0 items-center justify-center">
      <Notch :state="notch" />
      <button v-if="ready && started" type="button" class="round absolute end-0" aria-label="Settings" @click="settingsOpen = true">
        <Glyph name="settings" class="size-5" />
      </button>
    </header>

    <!-- Cannot run here -->
    <main v-if="framed || !webCryptoSupported" class="flex flex-1 flex-col items-center justify-center pb-16 text-center">
      <span class="flex size-16 items-center justify-center rounded-full bg-bad-bg text-bad">
        <Glyph name="alert" fill class="size-8" />
      </span>
      <h1 class="display mt-5 text-[26px] font-medium">{{ framed ? "Open Oeca Hop in its own tab" : "This browser can't run Oeca Hop" }}</h1>
      <p class="mt-2 max-w-[18rem] text-[15px] leading-snug text-ink-2">
        {{ framed ? "It doesn't run inside another page." : "Try Safari 17, Chrome 113 or newer, on https." }}
      </p>
    </main>

    <!-- First visit (or a changed notice): nothing connects until Start -->
    <main v-else-if="!started" class="flex flex-1 flex-col" aria-labelledby="intro-title">
      <IdentityCard class="rise mx-auto mt-4" name="This browser" caption="Off until you start" />

      <h1 id="intro-title" class="rise rise-2 display mt-9 text-center text-[34px] font-medium leading-[1.05] tracking-[-0.03em] min-[400px]:text-[38px]">
        Send files<br />to your PC
      </h1>
      <p class="rise rise-2 sub mt-3 text-center text-[16px]">No app. No QR code. No IP address.</p>

      <!-- Three facts, each a button to the details -->
      <ul class="rise rise-3 mx-auto mt-8 grid w-full max-w-[20rem] grid-cols-3 gap-2">
        <li v-for="f in FACTS" :key="f.label">
          <button type="button" class="press flex w-full flex-col items-center gap-2.5 rounded-[20px] py-1" @click="privacyOpen = true">
            <span class="object flex size-14 items-center justify-center rounded-full">
              <Glyph :name="f.icon" fill class="size-6" />
            </span>
            <span class="text-[13px] font-medium leading-tight">{{ f.label }}</span>
          </button>
        </li>
      </ul>

      <!-- In reach: the bar stays at the bottom -->
      <div class="sticky bottom-0 -mx-4 mt-auto bg-gradient-to-t from-bg from-60% to-bg/0 px-4 pt-8 pb-[max(0.25rem,env(safe-area-inset-bottom))] sm:static sm:mx-0 sm:bg-none sm:px-0">
        <div class="flex gap-2">
          <button type="button" class="pill-soft w-[38%]" @click="privacyOpen = true">Privacy</button>
          <button type="button" class="pill-cta flex-1" @click="consent">Start Hop</button>
        </div>
        <p class="sub mt-3 text-center text-[12px] leading-snug">Starting connects this page to LocalSend's public server</p>
      </div>
    </main>

    <!-- Discovery -->
    <main v-else class="flex flex-1 flex-col">
      <button type="button" class="rise press mx-auto mt-4 block w-full max-w-[21rem] rounded-[24px]" aria-label="Rename this browser" @click="settingsOpen = true">
        <IdentityCard :name="alias || '…'" caption="How your PC sees this browser" :digits="digits" editable />
      </button>

      <div class="mt-5 space-y-2 empty:hidden">
        <p v-if="identityNotice" class="callout bg-object text-ink-2 shadow-[0_0_0_1px_rgb(var(--line)/0.06)]" role="status">
          <Glyph name="info" fill class="mt-0.5 size-5 shrink-0 text-ink-3" />{{ identityNotice }}
        </p>
        <div v-if="offline" class="callout items-center bg-warn-bg text-warn" role="alert">
          <Glyph name="alert" fill class="size-5 shrink-0" />
          <p class="min-w-0 flex-1">{{ store.limited ? "Server busy. Retrying in about 15 min." : "Connection lost. Reconnecting…" }}</p>
          <button v-if="!store.limited" type="button" class="pill-sm -me-1.5 bg-warn/15" @click="retryNow">Retry</button>
        </div>
      </div>

      <!-- Devices: one card, a row each -->
      <section class="rise rise-2 object mt-5 rounded-[28px] p-2" aria-labelledby="devices-title">
        <div class="flex items-baseline justify-between px-3 pt-2 pb-1">
          <h2 id="devices-title" class="display text-[18px] font-medium tracking-[-0.01em]">Devices</h2>
          <span v-if="store.peers.length" class="text-[14px] tabular-nums text-ink-3">{{ store.peers.length }}</span>
        </div>
        <TransitionGroup v-if="store.peers.length" tag="ul" name="arrive" appear>
          <li v-for="peer in store.peers" :key="peer.id">
            <button
              type="button"
              class="press group flex w-full items-center gap-3 rounded-[20px] px-2 py-2 text-start hover:bg-ink/[0.03] active:bg-ink/[0.05]"
              :aria-label="`Send files to ${peer.alias || 'Unnamed device'}, ${peerDetail(peer.deviceType, peer.deviceModel)}`"
              @click="selectPeer(peer)"
            >
              <span class="flex size-12 shrink-0 items-center justify-center rounded-full bg-ink text-bg" aria-hidden="true">
                <Glyph :name="GLYPH[peerKind(peer.deviceType).icon] ?? 'cube'" fill class="size-6" />
              </span>
              <span class="min-w-0 flex-1">
                <span class="block truncate text-[16px] font-medium">{{ peer.alias || "Unnamed device" }}</span>
                <span class="sub block truncate text-[13px]">{{ peerDetail(peer.deviceType, peer.deviceModel) }}</span>
              </span>
              <span class="flex size-9 shrink-0 items-center justify-center rounded-full bg-ink/[0.06] text-ink-2 transition-colors duration-200 group-hover:bg-ink group-hover:text-bg" aria-hidden="true">
                <Glyph name="arrow" class="size-[18px] transition-transform duration-500 [transition-timing-function:var(--spring)] group-hover:-translate-y-px group-hover:translate-x-px" />
              </span>
            </button>
          </li>
        </TransitionGroup>
        <div v-else class="flex items-center gap-3 px-2 py-2">
          <span class="flex size-12 shrink-0 items-center justify-center rounded-full bg-ink/[0.06] text-ink-3" aria-hidden="true">
            <Glyph :name="store.signaling ? 'desktop' : 'arc'" class="size-[22px]" :class="{ spin: !store.signaling && !offline }" />
          </span>
          <span class="min-w-0 flex-1">
            <span class="block text-[16px] font-medium">{{ store.signaling ? "Looking for your PC" : offline ? "Offline" : "Connecting…" }}</span>
            <span class="sub block text-[13px] leading-snug">
              {{ store.signaling ? "Open Oeca and turn on Hop. Same Wi-Fi, no VPN, works best." : "Reaching the signaling server" }}
            </span>
          </span>
        </div>
      </section>

      <button type="button" class="press sub mx-auto mt-auto flex items-center gap-1.5 rounded-full px-4 pt-8 pb-1 text-[13px]" @click="privacyOpen = true">
        <Glyph name="lock" class="size-3.5" />Encrypted, and nothing arrives unasked
      </button>
    </main>

    <SendSheet :peer="sending" @close="closeSend" @send="send" />
    <SessionDialog />
    <SettingsSheet
      :visible="settingsOpen"
      :alias="alias"
      :online="!!store.signaling"
      :busy="transferRunning"
      @close="settingsOpen = false"
      @rename="rename"
      @turn-off="turnOff"
    />
    <PrivacySheet :visible="privacyOpen" :alias="started ? alias : ''" @close="privacyOpen = false" />
    <ConfirmDialog />
  </div>
</template>

<script setup lang="ts">
import { PeerDeviceType, type ClientInfo } from "@/services/signaling";
import {
  explain,
  markSelecting,
  retryNow,
  selectionCancelled,
  setupConnection,
  startSendSession,
  store,
  updateAliasState,
} from "@/services/store";
import { isFinal } from "~/services/transfer";
import { canonicalFingerprint, loadCertificate, stableAlias } from "~/services/trust";
import { shortFingerprint } from "~/services/devices";
import { getAgentInfoString } from "~/utils/userAgent";
import { protocolVersion } from "~/services/webrtc";
import { generateRandomAlias } from "~/utils/alias";
import SessionDialog from "~/components/dialog/SessionDialog.vue";
import { noticeId, notchState, peerDetail, peerKind, statusLine } from "~/utils/view";
import { ask } from "~/utils/confirm";
import {
  generateClientTokenFromCurrentTimestamp,
  generateKeyPair,
  isWebCryptoSupported,
  upgradeToEd25519IfSupported,
} from "~/services/crypto";

definePageMeta({
  title: "index.seo.title",
  description: "index.seo.description",
});

const runtimeConfig = useRuntimeConfig();

const GLYPH: Record<string, "phone" | "desktop" | "globe"> = { phone: "phone", computer: "desktop", browser: "globe" };
const FACTS = [
  { icon: "lock", label: "Encrypted" },
  { icon: "shield", label: "You approve" },
  { icon: "nearby", label: "Public server" },
] as const;
/** oeca: the notice accepted here (a changed notice asks again). */
const CONSENT_KEY = "oeca-nearby-consent";

// oeca: inside another page's frame, its clicks could pass for the user's
const framed = ref(false);
const webCryptoSupported = ref(true);
const ready = ref(false);
const started = ref(false);
const settingsOpen = ref(false);
const privacyOpen = ref(false);
const sending = ref<ClientInfo | null>(null);
const identityNotice = ref("");

const alias = computed(() => store.client?.alias ?? store._proposingClient?.alias ?? "");
const status = computed(() =>
  statusLine({
    framed: framed.value,
    crypto: webCryptoSupported.value,
    started: started.value,
    link: store.link,
    online: !!store.signaling,
    limited: store.limited,
    peers: store.peers.length,
  }),
);
const notch = computed(() => notchState(status.value, started.value, store.session.transfer, store.session.fileState));
const offline = computed(() => started.value && !store.signaling && (store.link === "reconnecting" || store.limited));
/** The end of this browser's key fingerprint, as a PC lists it. */
const digits = computed(() => {
  const f = store.certificate?.getFingerprints()[0];
  const fp = f ? canonicalFingerprint(`${f.algorithm} ${f.value}`) : null;
  return fp ? shortFingerprint(fp).slice(-4) : "";
});
const transferRunning = computed(() => {
  const t = store.session.transfer;
  return !!t && !isFinal(t.stage);
});

function selectPeer(peer: ClientInfo) {
  // The device exactly as shown; a running transfer keeps the page busy
  if (!markSelecting({ id: peer.id, alias: peer.alias })) {
    store.session.message = "Another transfer is in progress.";
    return;
  }
  sending.value = peer;
}

function closeSend() {
  sending.value = null;
  selectionCancelled();
}

async function send(files: File[]) {
  const peer = sending.value;
  sending.value = null;
  if (!peer) return;
  try {
    // Checked again here: it must still be the device shown (id and name)
    await startSendSession({ files, peer: { id: peer.id, alias: peer.alias } });
  } catch (e) {
    selectionCancelled();
    store.session.message = explain(e);
  }
}

function rename(next: string) {
  const current = store.client;
  if (!current || !store.signaling) return;
  store.signaling.send({
    type: "UPDATE",
    info: {
      alias: next,
      version: current.version,
      deviceModel: current.deviceModel,
      deviceType: current.deviceType,
      token: current.token,
    },
  });
  updateAliasState(next);
}

function storage(): Storage | null {
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function consent() {
  storage()?.setItem(CONSENT_KEY, noticeId());
  void start();
}

async function turnOff() {
  const { yes } = await ask({
    icon: "power",
    title: "Turn off Hop?",
    message: "This page disconnects until you start Hop again. Trusted devices are kept.",
    confirm: "Turn off",
    destructive: true,
  });
  if (!yes) return;
  storage()?.removeItem(CONSENT_KEY);
  location.reload();
}

async function start() {
  if (started.value) return;
  started.value = true;

  await upgradeToEd25519IfSupported();

  store.key = await generateKeyPair();

  // oeca: the persistent DTLS identity (peers verify and remember it)
  try {
    const id = await loadCertificate();
    store.certificate = markRaw(id.cert);
    store.persistentIdentity = id.state !== "unsaved";
    const until = new Date(id.expires).toLocaleDateString();
    identityNotice.value = {
      kept: id.endingSoon ? `This browser's identity ends on ${until}; your devices will verify it again then.` : "",
      new: "New browser: your devices will ask you to verify it once.",
      renewed: "This browser's identity was renewed: your devices will verify it again.",
      unsaved: "Private browsing: your devices will verify this browser every time.",
    }[id.state];
  } catch (e) {
    console.warn("No DTLS certificate; a new one per connection:", (e as Error)?.name);
    identityNotice.value = "No identity could be created here: devices can't verify this browser.";
  }

  const token = await generateClientTokenFromCurrentTimestamp(store.key);

  const info = {
    alias: stableAlias(generateRandomAlias),
    version: protocolVersion,
    deviceModel: getAgentInfoString(navigator.userAgent),
    deviceType: PeerDeviceType.web,
    token: token,
  };

  await setupConnection({
    url: runtimeConfig.public.signalingUrl,
    stunServers: runtimeConfig.public.stunServers as string[],
    info,
  });
}

onMounted(() => {
  if (window.top !== window.self) {
    framed.value = true;
    return;
  }
  // oeca: Nearby has no service worker. One left on this origin (an older
  // build, a test) could keep serving outdated code: it is removed. Site data
  // (identity, trusted devices) is not touched
  void navigator.serviceWorker?.getRegistrations().then((all) => all.forEach((r) => void r.unregister()));
  webCryptoSupported.value = isWebCryptoSupported();
  if (!webCryptoSupported.value) {
    console.error("Web Crypto API is not supported in this browser.");
    return;
  }
  ready.value = true;
  // Accepted before (this notice): on, as it was left
  if (storage()?.getItem(CONSENT_KEY) === noticeId()) void start();
});
</script>
