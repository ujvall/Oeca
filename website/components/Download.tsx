import Image from "next/image";
import { ArrowUpRight } from "@phosphor-icons/react/dist/ssr";
import { WindowsMark } from "./WindowsMark";
import { DOWNLOAD_URL, LICENSE_URL, RELEASES_URL, REQUIREMENTS, VERSION } from "@/lib/site";

export function Download() {
  return (
    <section id="download" className="section scroll-mt-14" aria-labelledby="download-title">
      <div className="shell">
        <div data-reveal className="panel grid gap-8 p-6 md:grid-cols-[1fr_auto] md:items-end md:gap-12 md:p-10">
          <div>
            <Image src="/media/oeca-icon-256.webp" alt="" width={48} height={48} className="size-12" />
            <h2 id="download-title" className="t-title mt-6">
              Get Oeca for Windows
            </h2>
            <p className="t-body mt-3 max-w-[30rem]">
              Free and open source under the{" "}
              <a className="link" href={LICENSE_URL}>
                MIT License
              </a>
              . Installs for your account, with no administrator rights. {REQUIREMENTS}. Not code-signed yet, so
              SmartScreen may ask you to confirm (see the{" "}
              <a className="link" href="/code-signing/">
                code signing policy
              </a>
              ).
            </p>
          </div>
          <div className="flex flex-col gap-3 sm:flex-row">
            <a className="btn btn-primary" href={DOWNLOAD_URL}>
              <WindowsMark size={14} />
              Download {VERSION}
            </a>
            <a className="btn btn-ghost group" href={RELEASES_URL}>
              All releases
              <ArrowUpRight
                size={16}
                aria-hidden
                className="text-muted transition-[transform,color] duration-200 ease-out group-hover:-translate-y-0.5 group-hover:translate-x-0.5 group-hover:text-paper"
              />
            </a>
          </div>
        </div>
      </div>
    </section>
  );
}
