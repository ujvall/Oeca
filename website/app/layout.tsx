import type { Metadata, Viewport } from "next";
import { SITE_URL } from "@/lib/site";
import { Motion } from "@/components/Motion";
import "./globals.css";

const description =
  "A native notch for the top of your Windows desktop: the time, what's playing, a timer and your clipboard history, one click away.";

export const metadata: Metadata = {
  metadataBase: new URL(SITE_URL),
  title: "Oeca: a native notch for Windows",
  description,
  applicationName: "Oeca",
  icons: {
    icon: [
      { url: "/favicon.ico", sizes: "any" },
      { url: "/media/oeca-icon-32.png", type: "image/png", sizes: "32x32" },
      { url: "/media/oeca-icon-64.png", type: "image/png", sizes: "64x64" },
    ],
    apple: "/media/oeca-icon-180.png",
  },
  openGraph: {
    type: "website",
    title: "Oeca: a native notch for Windows",
    description,
    siteName: "Oeca",
  },
  twitter: {
    card: "summary_large_image",
    title: "Oeca: a native notch for Windows",
    description,
  },
};

export const viewport: Viewport = {
  themeColor: "#0b0b0c",
  colorScheme: "dark",
};

// Runs before first paint: hides intro elements only when motion is allowed,
// and un-hides them after 2.5 s if the animation never starts.
const motionGate = `(function(){var d=document.documentElement;if(!matchMedia('(prefers-reduced-motion: reduce)').matches){d.classList.add('motion');setTimeout(function(){if(!window.__oecaIntro)d.classList.remove('motion')},2500)}})();`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: motionGate }} />
        {/* Geist by Vercel, via Google Fonts (SIL OFL): the site's only family */}
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="" />
        <link
          rel="stylesheet"
          href="https://fonts.googleapis.com/css2?family=Geist:wght@400;500;600&display=swap"
        />
      </head>
      <body>
        {children}
        <Motion />
      </body>
    </html>
  );
}
