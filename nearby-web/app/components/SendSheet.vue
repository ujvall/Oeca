<!-- oeca: sending to one device: the destination, the files chosen with the
     browser's own picker (any number of picks, each file removable), and
     Send. Nothing starts before Send; the device is checked again then
     (store.ts: it must still be the one shown). -->
<template>
  <Dialog :visible="!!props.peer" labelledby="send-title" dismissible @close="emit('close')">
    <template #header>
      <SheetHeader id="send-title" :title="`Send to ${props.peer?.alias || 'Unnamed device'}`" :subtitle="detail">
        <template #start>
          <button type="button" class="round" aria-label="Close" @click="emit('close')"><Glyph name="close" class="size-5" /></button>
        </template>
        <template #end>
          <button v-if="files.length" type="button" class="round" aria-label="Add files" @click="choose"><Glyph name="plus" class="size-5" /></button>
        </template>
      </SheetHeader>
    </template>

    <input ref="picker" type="file" multiple class="sr-only" tabindex="-1" aria-hidden="true" @change="picked" />

    <!-- Nothing chosen: one big target -->
    <button v-if="files.length === 0" type="button" class="dots press relative mt-2 flex h-[248px] w-full flex-col items-center justify-end rounded-[24px] bg-sunk/60 pb-7" @click="choose">
      <!-- Any kind of file: photos, documents, videos -->
      <span class="absolute inset-x-0 top-6 h-[120px]" aria-hidden="true">
        <span v-for="(s, i) in STICKERS" :key="s.icon" class="absolute" :class="s.at"><span class="drift block" :class="['', 'drift-2', 'drift-3'][i]">
          <Glyph :name="s.icon" fill class="sticker size-12" />
          <span class="absolute -end-1.5 -bottom-1.5 flex size-5 items-center justify-center rounded-full bg-[#111214] text-white ring-2 ring-white">
            <Glyph name="plus" class="size-3" />
          </span>
        </span></span>
      </span>
      <span class="display text-[20px] font-medium tracking-[-0.01em]">Photos, videos, documents</span>
      <span class="sub mt-1 text-[14px]">Up to {{ MAX_SEND_FILES }} files</span>
    </button>

    <template v-else>
      <div class="dots mt-2 rounded-[24px] bg-sunk/60 pt-3 pb-4">
        <FileStack :files="files" />
        <p class="mt-2 text-center text-[15px] font-medium tabular-nums">
          {{ countLabel(files.length) }} <span class="text-ink-3">· {{ totalSize(files) }}</span>
        </p>
      </div>
      <TransitionGroup tag="ul" name="arrive" class="list-group mt-4">
        <li v-for="(f, i) in files" :key="key(f)" class="row min-h-[52px] gap-2 pe-1.5">
          <FileName :name="f.name" class="min-w-0 flex-1 text-[15px]" />
          <span class="shrink-0 tabular-nums text-[13px] text-ink-3">{{ formatBytes(f.size) }}</span>
          <button type="button" class="press flex size-10 shrink-0 items-center justify-center rounded-full text-ink-3 active:bg-ink/[0.07]" :aria-label="`Remove ${f.name}`" @click="files.splice(i, 1)">
            <Glyph name="close" class="size-[18px]" />
          </button>
        </li>
      </TransitionGroup>
    </template>

    <div class="mt-3 space-y-2">
      <p v-if="issues.blocking && files.length" class="callout bg-bad-bg text-bad" role="alert">
        <Glyph name="alert" fill class="mt-0.5 size-5 shrink-0" />{{ issues.blocking }}
      </p>
      <p v-for="note in issues.notes" :key="note" class="callout bg-warn-bg text-warn" role="status">
        <Glyph name="alert" fill class="mt-0.5 size-5 shrink-0" />{{ note }}
      </p>
    </div>

    <template #footer>
      <button
        type="button"
        class="pill-cta w-full"
        :disabled="files.length > 0 && !!issues.blocking"
        @click="files.length ? emit('send', [...files]) : choose()"
      >
        <Glyph v-if="files.length" name="send" class="size-5" />
        {{ files.length ? `Send ${countLabel(files.length)}` : "Choose files" }}
      </button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import type { ClientInfo } from "~/services/signaling";
import { formatBytes } from "~/utils/fileSize";
import { countLabel, MAX_SEND_FILES, peerDetail, selectionIssues, totalSize } from "~/utils/view";

const props = defineProps<{ peer: ClientInfo | null }>();
const emit = defineEmits<{ close: []; send: [files: File[]] }>();

const picker = ref<HTMLInputElement | null>(null);
const files = ref<File[]>([]);
const issues = computed(() => selectionIssues(files.value));
const detail = computed(() => (props.peer ? peerDetail(props.peer.deviceType, props.peer.deviceModel) : ""));

const STICKERS = [
  { icon: "image", at: "start-[18%] top-6 -rotate-[10deg]" },
  { icon: "file", at: "start-1/2 top-0 -translate-x-1/2 rotate-[4deg]" },
  { icon: "film", at: "end-[18%] top-8 rotate-[12deg]" },
] as const;
const key = (f: File) => `${f.name}\u0000${f.size}\u0000${f.lastModified}`;

// A new destination starts empty
watch(
  () => props.peer?.id,
  () => (files.value = []),
);

function choose() {
  picker.value?.click();
}

function picked(e: Event) {
  const input = e.target as HTMLInputElement;
  const seen = new Set(files.value.map(key));
  for (const f of Array.from(input.files ?? [])) {
    if (!seen.has(key(f))) {
      files.value.push(f);
      seen.add(key(f));
    }
  }
  // The same file can be picked again after removing it
  input.value = "";
}
</script>
