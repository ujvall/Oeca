<!-- Modified for Oeca Nearby from LocalSend Web (Apache-2.0): see OECA.md. -->
<!-- oeca: one sheet for the whole transfer: who the other device is (proven,
     never from its name), what it asks, the files and what became of each.
     Every decision names the transfer it was shown for (store.ts checks it),
     and counts only once the question has been on screen (ARMING_MS). -->
<template>
  <Dialog :visible="visible" labelledby="session-title">
    <template #header>
      <SheetHeader id="session-title" :title="transfer ? title : 'Oeca Hop'" :subtitle="transfer ? `${transfer.direction === 'receive' ? 'From' : 'To'} ${name}` : ''" />
      <div v-if="badge" class="mt-2.5 flex justify-center">
        <span class="inline-flex items-center gap-1.5 rounded-full px-3 py-1.5 text-[13px] font-semibold" :class="BADGE[badge.tone]">
          <Glyph v-if="badge.tone === 'good'" name="shield" class="size-4" />
          <Glyph v-else-if="badge.tone !== 'plain'" name="alert" class="size-4" />
          {{ badge.label }}
        </span>
      </div>
      <p v-if="transfer && trusted?.nameMismatch" class="mt-1.5 text-center text-[13px] text-ink-3">Now calls itself “{{ transfer.claimed.peer.alias }}”</p>
    </template>

    <span class="sr-only" aria-live="assertive">{{ transfer ? title : "" }}</span>

    <!-- Identity warnings: before any decision, never hidden -->
    <div v-if="warning" class="callout mt-2" :class="warning.bad ? 'bg-bad-bg text-bad' : 'bg-warn-bg text-warn'" role="alert">
      <Glyph name="alert" fill class="mt-0.5 size-5 shrink-0" />
      <p><span class="font-semibold">{{ warning.title }}</span> {{ warning.text }}</p>
    </div>

    <!-- Waiting for the other side: your disc, working -->
    <div v-if="waiting" class="flex flex-col items-center py-4">
      <Radar :searching="true" />
      <p class="mt-1 text-[15px] text-ink-2">{{ STAGE_NOTE[stage!] }}</p>
    </div>

    <!-- Only an authenticated connection has a code -->
    <template v-if="code">
      <p class="mt-5 text-center text-[15px] text-ink-2">{{ trusted ? "Verified device" : "Same code on your PC?" }}</p>
      <div class="mt-3 flex items-center justify-center gap-3" :aria-label="`Code ${spokenCode}`" role="img">
        <div v-for="(g, n) in [digits.slice(0, 3), digits.slice(3)]" :key="n" class="flex gap-1.5">
          <span v-for="(d, i) in g.split('')" :key="i" class="object flex h-[64px] w-[min(44px,12vw)] items-center justify-center rounded-[14px] text-[32px] font-semibold tabular-nums">{{ d }}</span>
        </div>
      </div>
    </template>
    <div v-if="stage === 'verification'" class="mt-6">
      <label for="device-name" class="px-4 text-[13px] font-medium text-ink-3">Name this device</label>
      <input id="device-name" v-model="name_" maxlength="64" autocomplete="off" enterkeyhint="done" class="field mt-1.5" />
    </div>

    <!-- Incoming: everything that is offered -->
    <template v-if="stage === 'approval' && transfer">
      <FileStack :files="offeredNames" class="mt-1" />
      <p class="mt-2 text-center text-[15px] font-medium tabular-nums">
        {{ countLabel(transfer.offered.length) }} <span class="text-ink-3">· {{ offeredSize }}</span>
      </p>
      <div v-if="risky" class="callout mt-4 bg-warn-bg text-warn" role="alert">
        <Glyph name="alert" fill class="mt-0.5 size-5 shrink-0" />
        <p><span class="font-semibold">{{ risky }} can run programs.</span> Open {{ risky === 1 ? "it" : "them" }} only if you expected {{ risky === 1 ? "it" : "them" }}.</p>
      </div>
      <ul class="list-group mt-3">
        <li v-for="f in transfer.offered" :key="f.id" class="row min-h-[52px]">
          <FileName :name="f.fileName" class="min-w-0 flex-1 text-[15px]" />
          <span class="shrink-0 tabular-nums text-[13px] text-ink-3">{{ formatBytes(f.size) }}</span>
        </li>
      </ul>
      <p class="mt-3 text-center text-[12px] text-ink-3">Kept in this browser until you save them. Never opened.</p>
    </template>

    <!-- Progress, then each file's outcome -->
    <template v-if="showFiles">
      <ProgressRing class="mt-3" :progress="overall" :state="stage === 'done' ? 'done' : busy ? 'moving' : 'ended'" label="Overall progress" />
      <p class="mt-3 text-center text-[15px] font-medium tabular-nums">
        {{ confirmed.done }} of {{ confirmed.all }} <span class="text-ink-3">{{ transfer?.direction === "send" ? "confirmed" : "received" }}</span>
      </p>
      <ul class="list-group mt-5">
        <FileProgress v-for="row in rows" :key="row.id" :row="row" />
      </ul>
    </template>

    <p v-if="failure" class="callout mt-3 bg-group text-ink-2" role="status">
      <Glyph name="info" fill class="mt-0.5 size-5 shrink-0" />{{ failure }}
    </p>
    <p v-if="store.session.message" class="callout mt-3 bg-group text-ink" role="status">
      <Glyph name="info" fill class="mt-0.5 size-5 shrink-0 text-ink-3" />{{ store.session.message }}
    </p>

    <!-- Received: the user saves each file (a tap is the gesture every browser wants) -->
    <template v-if="store.session.received.length">
      <ul class="list-group mt-4">
        <li v-for="(r, i) in store.session.received" :key="i" class="row min-h-[56px] pe-2">
          <FileName :name="r.name" class="min-w-0 flex-1 text-[15px] font-medium" />
          <button type="button" class="pill-sm" @click="save(i, r.file)">
            <Glyph :name="saved.has(i) ? 'check' : 'download'" class="size-4" />
            {{ saved.has(i) ? "Save again" : "Save" }}
          </button>
        </li>
      </ul>
      <p class="mt-3 text-center text-[12px] leading-snug text-ink-3">Saves to your browser's downloads. Unsaved files are removed when you close this.</p>
    </template>

    <p v-if="busy && store.session.wasHidden" class="callout mt-3 bg-warn-bg text-warn" role="alert">
      <Glyph name="alert" fill class="mt-0.5 size-5 shrink-0" />Keep this page open until it finishes.
    </p>

    <template #footer>
      <div v-if="transfer && asks" class="grid gap-2" :class="acceptLabel ? 'grid-cols-2 max-[359px]:grid-cols-1' : 'grid-cols-1'">
        <button type="button" class="pill-soft" :disabled="!armed" @click="act(transfer.id, (id) => respond(id, false))">{{ refuseLabel }}</button>
        <button v-if="acceptLabel" type="button" class="max-[359px]:order-first" :class="transfer.capability === 'stock' ? 'pill-warn' : 'pill-cta'" :disabled="!armed" @click="accept(transfer.id)">
          {{ acceptLabel }}
        </button>
      </div>
      <button v-else-if="transfer && busy" type="button" class="pill-soft w-full" @click="cancelSession(transfer.id)">Cancel</button>
      <div v-else class="flex flex-col gap-2">
        <button type="button" class="pill-cta w-full" @click="dismissSession()">Done</button>
        <button v-if="transfer && code && trusted" type="button" class="pill-soft w-full text-bad" @click="act(transfer.id, forgetPeer)">Forget this device</button>
      </div>
    </template>
  </Dialog>
</template>
<script setup lang="ts">
import {
  allowOnce,
  cancelSession,
  confirmName,
  dismissSession,
  displayed,
  explain,
  failureText,
  forgetPeer,
  respond,
  store,
  verifyPeer,
} from "~/services/store";
import type { Stage } from "~/services/transfer";
import { dangerous } from "~/services/limits";
import { formatBytes } from "~/utils/fileSize";
import { saveFile } from "~/utils/fileSink";
import {
  armsIn,
  confirmedCount,
  countLabel,
  fileRows,
  identityBadge,
  overallProgress,
  peerName,
  stageTitle,
  totalSize,
  type Tone,
} from "~/utils/view";

const BADGE: Record<Tone, string> = {
  good: "bg-good-bg text-good",
  plain: "bg-ink/[0.07] text-ink-2",
  warn: "bg-warn-bg text-warn",
  bad: "bg-bad-bg text-bad",
};

const STAGE_NOTE: Partial<Record<Stage, string>> = {
  signaling: "Waiting for an answer",
  connecting: "Securing a direct connection",
  "awaiting-offer": "Waiting for the files",
  "awaiting-peer": "Waiting for the other device",
};

const transfer = computed(() => store.session.transfer);
const stage = computed(() => transfer.value?.stage);
const visible = computed(() => transfer.value !== null || store.session.message !== "");
const busy = computed(() => !!stage.value && !["done", "declined", "cancelled", "failed"].includes(stage.value));
const asks = computed(() => ["verification", "name-check", "unverified", "approval"].includes(stage.value ?? ""));
const waiting = computed(() => !!stage.value && stage.value in STAGE_NOTE && !showFiles.value);
const identity = computed(() => transfer.value?.identity?.state);
const trusted = computed(() => {
  const i = transfer.value?.identity;
  return i?.state === "trusted" ? i : null;
});
const connected = computed(() => transfer.value?.connection.state === "connected");
const code = computed(() => {
  const c = transfer.value?.connection;
  return c?.state === "authenticated" ? c.code : "";
});
const digits = computed(() => code.value.replace(/\s/g, ""));
const spokenCode = computed(() => digits.value.split("").join(" "));
const failure = computed(() => (transfer.value ? failureText(transfer.value) : ""));
const badge = computed(() => (transfer.value ? identityBadge(transfer.value) : null));
const name = computed(() => (transfer.value ? peerName(transfer.value) : ""));
const title = computed(() => (transfer.value ? stageTitle(transfer.value) : ""));

const warning = computed((): { title: string; text: string; bad: boolean } | null => {
  if (identity.value === "revoked") {
    return { bad: true, title: "You forgot this device.", text: "Verify its code again only if you mean to trust it again." };
  }
  if (identity.value === "changed") {
    return { bad: true, title: "Not the device you know by this name.", text: "If the codes differ, someone may be in between." };
  }
  if (stage.value === "name-check") {
    return { bad: false, title: "Your trusted device has a new name.", text: "Continue only if it's the one you meant." };
  }
  if (connected.value && identity.value === "new" && transfer.value?.capability === "stock") {
    return {
      bad: false,
      title: "Can't be verified.",
      text: "It isn't Oeca Hop (a LocalSend app, or a tampered connection). Allow it only if you expect it, for this transfer only.",
    };
  }
  return null;
});

const files = computed(() => store.session.fileState);
const showFiles = computed(
  () => ["awaiting-peer", "transferring", "done", "failed", "cancelled"].includes(stage.value ?? "") && Object.keys(files.value).length > 0,
);
const rows = computed(() => (transfer.value ? fileRows(files.value, transfer.value.direction, transfer.value.stage) : []));
const overall = computed(() => (transfer.value ? overallProgress(files.value, transfer.value.direction) : 0));
const confirmed = computed(() => confirmedCount(files.value));
const risky = computed(() => transfer.value?.offered.filter((f) => dangerous(f.fileName)).length ?? 0);
const offeredSize = computed(() => totalSize(transfer.value?.offered ?? []));
const offeredNames = computed(() => (transfer.value?.offered ?? []).map((f) => ({ name: f.fileName })));

const refuseLabel = computed(() => {
  if (stage.value === "verification") return "Codes differ";
  if (stage.value === "name-check") return "Not this one";
  return stage.value === "approval" ? "Decline" : "Don't send";
});
// Only what the trust layer allows here (never a weaker path)
const acceptLabel = computed(() => {
  const t = transfer.value;
  if (!t) return "";
  if (stage.value === "verification") return "Codes match";
  if (stage.value === "name-check") return "It's this one";
  if (t.capability === "stock" && identity.value === "new") {
    return stage.value === "approval" ? "Accept once" : "Send once";
  }
  if (stage.value === "approval" && trusted.value) return "Accept";
  return "";
});

function accept(id: string) {
  if (stage.value === "verification") return act(id, (i) => verifyPeer(i, name_.value));
  if (stage.value === "name-check") return act(id, confirmName);
  if (transfer.value?.capability === "stock") return act(id, allowOnce);
  return act(id, (i) => respond(i, true));
}

const name_ = ref("");
watch(
  () => transfer.value?.id,
  () => (name_.value = transfer.value?.claimed.peer.alias ?? ""),
  { immediate: true },
);
// A question counts as shown in the frame that painted it (none in a hidden tab)
watch(
  () => [transfer.value?.id, stage.value] as const,
  ([id, at]) => {
    if (id && at) requestAnimationFrame(() => displayed(id, at));
  },
  { immediate: true, flush: "post" },
);
// A new stage starts at the top (not where the last one was scrolled)
watch(
  stage,
  () => document.getElementById("session-title")?.closest(".sheet-panel")?.querySelector(".overflow-y-auto")?.scrollTo({ top: 0 }),
  { flush: "post" },
);

// The decision buttons wake when the question arms (store.ts refuses earlier taps anyway)
const now = ref(0);
let timer: ReturnType<typeof setTimeout> | undefined;
const armed = computed(() => {
  const t = transfer.value;
  if (!t) return false;
  return armsIn(t, now.value) === 0;
});
watch(
  () => [transfer.value?.id, transfer.value?.stage, transfer.value?.decideAfter] as const,
  () => {
    clearTimeout(timer);
    now.value = performance.now();
    const t = transfer.value;
    const left = t ? armsIn(t, now.value) : 0;
    if (left) timer = setTimeout(() => (now.value = performance.now()), left + 10);
  },
  { immediate: true },
);
onBeforeUnmount(() => clearTimeout(timer));

const saved = reactive(new Set<number>());
watch(
  () => transfer.value?.id,
  () => saved.clear(),
);
function save(i: number, file: File) {
  saveFile(file);
  saved.add(i);
}

async function act(id: string, decision: (id: string) => Promise<void>) {
  store.session.message = "";
  try {
    await decision(id);
  } catch (e) {
    store.session.message = explain(e);
  }
}
</script>
