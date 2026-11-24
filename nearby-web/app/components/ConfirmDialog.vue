<!-- oeca: a confirmation (utils/confirm.ts) as a small floating sheet: what
     happens, one line why, an optional switch, the action and Cancel. The
     safe choice is the focused one; a destructive action is red. -->
<template>
  <Teleport to="body">
    <Transition name="sheet" @after-enter="focusSafe">
      <div
        v-if="q"
        class="fixed inset-0 z-[60] flex items-end justify-center px-2 pb-[max(0.5rem,env(safe-area-inset-bottom))] sm:items-center sm:p-6"
        @keydown.esc.stop="answer(false)"
      >
        <div class="absolute inset-0 bg-black/40 backdrop-blur-[2px]" aria-hidden="true" @click="answer(false)" />
        <div
          class="sheet-panel relative w-full max-w-[24rem] rounded-[38px] bg-sheet px-3.5 pt-7 pb-3.5 text-center shadow-[0_30px_80px_-20px_rgb(0_0_0/0.5),0_0_0_1px_rgb(var(--line)/0.06)]"
          role="alertdialog"
          aria-modal="true"
          aria-labelledby="confirm-title"
          aria-describedby="confirm-message"
        >
          <span
            v-if="q.icon"
            class="mx-auto flex size-14 items-center justify-center rounded-full"
            :class="q.destructive ? 'bg-bad-bg text-bad' : 'bg-ink/[0.07] text-ink'"
            aria-hidden="true"
          >
            <Glyph :name="q.icon as any" fill class="size-7" />
          </span>
          <h2 id="confirm-title" class="display mt-4 px-3 text-[21px] font-medium leading-tight tracking-[-0.01em]">{{ q.title }}</h2>
          <p id="confirm-message" class="mx-auto mt-2 max-w-[19rem] px-3 text-[15px] leading-snug text-ink-2">{{ q.message }}</p>
          <label v-if="q.option" class="list-group mt-5 flex min-h-[56px] cursor-pointer items-center gap-3 rounded-[24px] bg-group px-4 text-start text-[15px] font-medium">
            <span class="min-w-0 flex-1">{{ q.option.label }}</span>
            <input v-model="option" type="checkbox" role="switch" class="peer sr-only" />
            <span
              class="relative h-[31px] w-[51px] shrink-0 rounded-full bg-ink/15 transition-colors duration-200 after:absolute after:start-[2px] after:top-[2px] after:size-[27px] after:rounded-full after:bg-white after:shadow after:transition-transform after:duration-200 peer-checked:bg-bad peer-checked:after:translate-x-5 peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-signal"
              aria-hidden="true"
            />
          </label>
          <div class="mt-5 flex flex-col gap-2">
            <button type="button" :class="q.destructive ? 'pill-bad' : 'pill-cta'" @click="answer(true, option)">{{ q.confirm }}</button>
            <button ref="safe" type="button" class="pill-soft" @click="answer(false)">Cancel</button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { answer, asking } from "~/utils/confirm";

const q = asking;
const option = ref(false);
const safe = ref<HTMLButtonElement | null>(null);
let before: Element | null = null;

function focusSafe() {
  safe.value?.focus({ preventScroll: true });
}

watch(asking, (now) => {
  if (typeof document === "undefined") return;
  if (now) {
    option.value = now.option?.value ?? false;
    before = document.activeElement;
    void nextTick(() => requestAnimationFrame(focusSafe));
  } else if (before instanceof HTMLElement) {
    before.focus({ preventScroll: true });
    before = null;
  }
});
</script>
