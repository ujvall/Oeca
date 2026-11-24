// oeca: the site's THIRD-PARTY-NOTICES.txt, written at every build from the
// packages whose code is actually in the bundle (Vite's module list), with
// each package's own license and notice files. Nothing to keep in step by hand.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const FILE = /^(licen[cs]e|copying|notice)/i;

/** The last client build's notices (nuxt.config.ts writes them into the site; no notices, no site). */
export let generated: string | null = null;
const RULE = "-".repeat(78);

/** The package folder a module id belongs to (its last node_modules/<name>), or null for this app's own. */
export function packageDir(id: string): string | null {
  const clean = id.replace(/^\0/, "").split("?")[0]!.replaceAll("\\", "/");
  const at = clean.lastIndexOf("/node_modules/");
  if (at < 0) return null;
  const [first, second] = clean.slice(at + "/node_modules/".length).split("/");
  if (!first) return null;
  return clean.slice(0, at + "/node_modules/".length) + (first.startsWith("@") ? `${first}/${second}` : first);
}

/** One package's entry: its name, version, license field, and every license / notice file it ships. */
export function entry(dir: string): { text: string; missing: boolean } {
  const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  const files = readdirSync(dir).filter((f) => FILE.test(f)).sort();
  const head = `${pkg.name} ${pkg.version} (${pkg.license ?? "no license field"})`;
  if (!files.length) return { text: `${RULE}\n${head}: NO LICENSE FILE SHIPPED (see its repository)\n${RULE}`, missing: true };
  const bodies = files.map((f) => `${RULE}\n${head}: ${f}\n${RULE}\n${readFileSync(join(dir, f), "utf8").replace(/\r\n/g, "\n").trimEnd()}`);
  return { text: bodies.join("\n\n"), missing: false };
}

/**
 * The Vite plugin. `always`: packages whose output ships without being a JS
 * module (Tailwind's generated CSS). `header`: what is not a package.
 */
export function notices({ header, always }: { header: string; always: string[] }) {
  let ssr = false;
  return {
    name: "oeca-notices",
    apply: "build" as const,
    configResolved(config: any) {
      ssr = !!config.build?.ssr;
    },
    generateBundle(this: any, _options: unknown, bundle: Record<string, any>) {
      if (ssr) return;
      const dirs = new Set(always);
      for (const chunk of Object.values(bundle)) {
        if (chunk.type !== "chunk") continue;
        for (const id of chunk.moduleIds as string[]) {
          const dir = packageDir(id);
          if (dir && existsSync(join(dir, "package.json"))) dirs.add(dir);
        }
      }
      const entries = [...dirs].map(entry).sort((a, b) => a.text.localeCompare(b.text));
      for (const e of entries) if (e.missing) this.warn(`no license file: ${e.text.split("\n")[1]}`);
      const list = entries.map((e) => `  ${e.text.split("\n")[1]!.split(":")[0]}`);
      const source = [header.trimEnd(), "", "Packages in this site:", ...new Set(list), "", ...entries.map((e) => e.text), ""].join("\n");
      generated = source;
    },
  };
}
