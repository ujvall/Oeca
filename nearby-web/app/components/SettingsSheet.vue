<!-- oeca: this browser's Nearby settings, as pages in one sheet: the root
     (name, look, the way to trusted devices and privacy, then what resets or
     stops Hop), trusted devices (devices.ts: forgetting is effective at
     once and reported only once saved), and the privacy notice. -->
<template>
  <Dialog :visible="props.visible" labelledby="settings-title" dismissible @close="close">
    <template #header>
      <SheetHeader id="settings-title" :title="TITLES[page]" :subtitle="page === 'devices' && list.length ? countDevices : ''">
        <template #start>
          <button v-if="page !== 'root'" type="button" class="round" aria-label="Back" @click="go('root')"><Glyph name="back" class="size-5" /></button>
        </template>
        <template #end>
          <button type="button" class="round" aria-label="Close" @click="close"><Glyph name="close" class="size-5" /></button>
        </template>
      </SheetHeader>
    </template>

    <Transition :name="direction">
      <!-- Root -->
      <div v-if="page === 'root'" key="root" class="space-y-4 pt-1">
        <form class="list-group p-1.5" @submit.prevent="rename">
          <div class="flex items-center gap-1.5">
            <label for="s-name" class="sr-only">This browser's name</label>
            <input id="s-name" v-model="alias" maxlength="64" autocomplete="off" enterkeyhint="done" :disabled="!props.online" class="field flex-1 bg-transparent px-3.5 text-[17px] font-semibold" />
            <button type="submit" class="pill-sm h-10 bg-cta px-4 text-on-cta" :disabled="!props.online || !alias.trim() || alias.trim() === props.alias">Save</button>
          </div>
        </form>
        <p class="sub -mt-2 px-4 text-[12px]">{{ props.online ? "What your PC sees. Not proof of who you are." : "Rename while connected." }}</p>

        <div class="segmented grid-cols-3" role="radiogroup" aria-label="Appearance">
          <span class="segment-thumb" :class="THUMB[MODES.findIndex((m) => m.value === mode)] ?? ''" aria-hidden="true" />
          <button v-for="m in MODES" :key="m.value" type="button" role="radio" :aria-checked="mode === m.value" class="segment" @click="mode = m.value">
            <Glyph :name="m.icon" class="size-4" />{{ m.label }}
          </button>
        </div>

        <div class="list-group">
          <button type="button" class="row press active:bg-ink/[0.04]" @click="go('devices')">
            <span class="tile-icon"><Glyph fill name="shield" class="size-[18px]" /></span>
            <span class="min-w-0 flex-1 text-[16px] font-medium">Trusted devices</span>
            <span class="text-[15px] tabular-nums text-ink-3">{{ trustedCount }}</span>
            <Glyph name="chevron" class="size-4 text-ink-3" />
          </button>
          <button type="button" class="row press active:bg-ink/[0.04]" @click="go('privacy')">
            <span class="tile-icon"><Glyph fill name="eye" class="size-[18px]" /></span>
            <span class="min-w-0 flex-1 text-[16px] font-medium">What Hop shares</span>
            <Glyph name="chevron" class="size-4 text-ink-3" />
          </button>
        </div>

        <div class="list-group">
          <button type="button" class="row press active:bg-bad/[0.06] disabled:opacity-40" :disabled="props.busy" @click="reset">
            <span class="tile-icon !bg-bad-bg !text-bad"><Glyph fill name="key" class="size-[18px]" /></span>
            <span class="min-w-0 flex-1">
              <span class="block text-[16px] font-medium text-bad">Reset identity</span>
              <span v-if="props.busy" class="block text-[12px] text-ink-3">Not during a transfer</span>
            </span>
          </button>
          <button type="button" class="row press active:bg-bad/[0.06]" @click="emit('turnOff')">
            <span class="tile-icon !bg-bad-bg !text-bad"><Glyph fill name="power" class="size-[18px]" /></span>
            <span class="min-w-0 flex-1 text-[16px] font-medium text-bad">Turn off Hop</span>
          </button>
        </div>

        <p v-if="message" class="px-4 text-center text-[13px] text-ink-2" role="status">{{ message }}</p>
        <p class="pb-1 text-center text-[12px] text-ink-3">
          Adapted from LocalSend Web ·
          <a href="/THIRD-PARTY-NOTICES.txt" class="underline decoration-line/30 underline-offset-2">Notices</a>
        </p>
      </div>

      <!-- Trusted devices -->
      <div v-else-if="page === 'devices'" key="devices" class="space-y-3 pt-1">
        <p v-if="storeNote" class="callout bg-warn-bg text-warn" role="status">
          <Glyph name="alert" fill class="mt-0.5 size-5 shrink-0" />{{ storeNote }}
        </p>
        <TransitionGroup v-if="list.length" tag="ul" name="arrive" class="list-group">
          <li v-for="d in list" :key="d.fingerprint" class="row min-h-[72px] py-2.5 pe-2.5">
            <span class="tile-icon" :class="d.state === 'trusted' ? '' : d.state === 'revoked' ? '!text-ink-3' : '!bg-warn-bg !text-warn'" aria-hidden="true">
              <Glyph fill :name="d.state === 'trusted' ? 'shield' : d.state === 'revoked' ? 'close' : 'alert'" class="size-[18px]" />
            </span>
            <span class="min-w-0 flex-1">
              <span class="block text-[16px] font-semibold leading-snug [overflow-wrap:anywhere]">{{ d.name }}</span>
              <span class="block text-[12px] text-ink-3">{{ STATE[d.state] }} · {{ new Date(d.verifiedAt).toLocaleDateString() }}</span>
              <span class="num block text-[11px] tracking-[0.06em] text-ink-3 [overflow-wrap:anywhere]" :aria-label="`Fingerprint ${d.short}`">{{ d.short }}</span>
            </span>
            <button
              v-if="d.state !== 'revoked'"
              type="button"
              class="pill-sm"
              :class="d.state === 'trusted' ? 'text-bad' : ''"
              :aria-label="`${d.state === 'trusted' ? 'Forget' : 'Retry forgetting'} ${d.name}`"
              @click="forget(d)"
            >
              {{ d.state === "trusted" ? "Forget" : "Retry" }}
            </button>
          </li>
        </TransitionGroup>
        <div v-else-if="!storeNote" class="flex flex-col items-center px-6 py-10 text-center">
          <span class="flex size-16 items-center justify-center rounded-full bg-group text-ink-3"><Glyph name="shield" class="size-8" /></span>
          <p class="display mt-4 text-[18px] font-medium">No trusted devices</p>
          <p class="sub mt-1 text-[14px]">Devices you verify with a code appear here</p>
        </div>
        <p v-if="message" class="px-4 text-center text-[13px] text-ink-2" role="status">{{ message }}</p>
        <button v-if="list.length || status === 'paused'" type="button" class="pill-soft w-full text-bad" @click="forgetAll">Forget all</button>
      </div>

      <!-- Privacy -->
      <div v-else key="privacy" class="pt-1">
        <PrivacyNotice :alias="props.alias" />
      </div>
    </Transition>
  </Dialog>
</template>

<script setup lang="ts">
import { useColorMode } from "@vueuse/core";
import { devices, TrustError, type Device, type DeviceState, type Status } from "~/services/devices";
import { explain, resetIdentity } from "~/services/store";
import { ask } from "~/utils/confirm";

const props = defineProps<{ visible: boolean; alias: string; online: boolean; busy: boolean }>();
const emit = defineEmits<{ close: []; rename: [alias: string]; turnOff: [] }>();

type Page = "root" | "devices" | "privacy";
const TITLES: Record<Page, string> = { root: "Settings", devices: "Trusted devices", privacy: "What Hop shares" };
// The selected segment's thumb: one column, plus the 4 px gap, per step
const THUMB = ["", "translate-x-[calc(100%+0.25rem)]", "translate-x-[calc(200%+0.5rem)]"];
const MODES = [
  { value: "auto", label: "System", icon: "contrast" },
  { value: "light", label: "Light", icon: "sun" },
  { value: "dark", label: "Dark", icon: "moon" },
] as const;
// oeca: no injected <style> while switching (the Content-Security-Policy refuses it)
const mode = useColorMode({ emitAuto: true, disableTransition: false });

const STATE: Record<DeviceState, string> = {
  trusted: "Trusted",
  revoked: "Forgotten",
  "revocation-pending": "Forgetting not saved yet",
};

const page = ref<Page>("root");
const direction = ref("push");
const alias = ref(props.alias);
const list = ref<Device[]>([]);
const status = ref<Status>("ready");
const message = ref("");

const trustedCount = computed(() => list.value.filter((d) => d.state === "trusted").length);
const countDevices = computed(() => `${trustedCount.value} trusted`);
const storeNote = computed(
  () =>
    ({
      ready: "",
      unavailable: "This browser can't keep trusted devices (private browsing?).",
      unreadable: "Trusted devices can't be read, so nobody is trusted for now.",
      paused: "A forgotten device isn't saved yet, so all trust is paused. Retry it, or forget all.",
    })[status.value],
);

function go(next: Page) {
  direction.value = next === "root" ? "pop" : "push";
  message.value = "";
  page.value = next;
}

function close() {
  emit("close");
}

async function refresh() {
  try {
    status.value = await devices.status();
    list.value = await devices.list();
  } catch {
    list.value = [];
  }
}

let unsubscribe: (() => void) | null = null;
watch(
  () => props.visible,
  (on) => {
    message.value = "";
    if (on) {
      page.value = "root";
      alias.value = props.alias;
      void refresh();
      unsubscribe = devices.onChange(() => void refresh());
    } else {
      unsubscribe?.();
      unsubscribe = null;
    }
  },
  { immediate: true },
);
watch(
  () => props.alias,
  (a) => {
    if (!alias.value) alias.value = a;
  },
);
onBeforeUnmount(() => unsubscribe?.());

function rename() {
  const next = alias.value.trim();
  if (next && next !== props.alias) {
    emit("rename", next);
    message.value = "Renamed.";
  }
}

async function forget(d: Device) {
  message.value = "";
  try {
    if (d.state === "trusted") await devices.forget(d.fingerprint);
    else await devices.retry();
    message.value = `${d.name} is forgotten. It will need its code again.`;
  } catch (e) {
    message.value = e instanceof TrustError ? explain(e) : "Couldn't forget it. Try again.";
  }
  await refresh();
}

async function forgetAll() {
  const { yes } = await ask({
    icon: "shield",
    title: "Forget all devices?",
    message: "Each one will need its code again. This can't be undone.",
    confirm: "Forget all",
    destructive: true,
  });
  if (!yes) return;
  message.value = "";
  try {
    await devices.forgetAll();
    message.value = "All devices are forgotten.";
  } catch (e) {
    message.value = e instanceof TrustError ? explain(e) : "Couldn't forget them. Try again.";
  }
  await refresh();
}

async function reset() {
  const { yes, option } = await ask({
    icon: "key",
    title: "Reset identity?",
    message: "Devices that trusted this browser will ask you to verify it again. The page reloads.",
    confirm: "Reset identity",
    destructive: true,
    option: { label: "Also forget trusted devices", value: false },
  });
  if (!yes) return;
  try {
    await resetIdentity(option);
    location.reload();
  } catch (e) {
    message.value = explain(e);
  }
}
</script>
