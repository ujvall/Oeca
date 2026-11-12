import { Nav } from "@/components/Nav";
import { Hero } from "@/components/Hero";
import { Features } from "@/components/Features";
import { Customize } from "@/components/Customize";
import { Technical } from "@/components/Technical";
import { Download } from "@/components/Download";
import { Footer } from "@/components/Footer";

export default function Home() {
  return (
    <>
      <a
        href="#features"
        className="sr-only z-50 rounded-lg bg-paper px-4 py-2 text-ink focus:not-sr-only focus:fixed focus:top-3 focus:left-3"
      >
        Skip to content
      </a>
      <Nav />
      <main>
        <Hero />
        <Features />
        <Customize />
        <Technical />
        <Download />
      </main>
      <Footer />
    </>
  );
}
