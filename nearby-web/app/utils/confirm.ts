// oeca: the page's own confirmation (instead of the browser's confirm()):
// one question at a time, answered by its buttons, Escape or a tap outside
// (both mean "no"). ConfirmDialog.vue shows it.

import { shallowRef } from "vue";

export type Question = {
  /** A Glyph name shown above the title. */
  icon?: string;
  title: string;
  message: string;
  /** The button that goes ahead, and whether it is destructive. */
  confirm: string;
  destructive?: boolean;
  /** An optional extra choice, answered with the question. */
  option?: { label: string; value: boolean };
};

type Pending = Question & { resolve: (answer: { yes: boolean; option: boolean }) => void };

export const asking = shallowRef<Pending | null>(null);

export function ask(q: Question): Promise<{ yes: boolean; option: boolean }> {
  asking.value?.resolve({ yes: false, option: false });
  return new Promise((resolve) => {
    asking.value = { ...q, resolve };
  });
}

export function answer(yes: boolean, option = false) {
  const q = asking.value;
  asking.value = null;
  q?.resolve({ yes, option });
}
