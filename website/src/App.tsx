import { useEffect, useState, type ReactNode } from "react";
import { content, siteConfig } from "./config";

type Releases = { macOS: boolean; Windows: boolean };
const noReleases: Releases = { macOS: false, Windows: false };

function platformSummary(releases: Releases) {
  const available = [releases.Windows && "Windows", releases.macOS && "macOS"].filter(Boolean);
  return available.length ? available.join(" + ") : "desktop";
}

function Logo({ compact = false }: { compact?: boolean }) {
  return (
    <a className={`logo ${compact ? "logoCompact" : ""}`} href="/" aria-label="NoBS PDF home">
      <img src="/brand/nobs-wordmark-lockup.svg" alt="NoBS PDF — Intelligent PDF Optimisation" />
    </a>
  );
}

function BuyButton({ available, className = "" }: { available: boolean; className?: string }) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  async function startCheckout() {
    setLoading(true);
    setError("");
    try {
      const response = await fetch("/api/checkout", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
      const data = await response.json();
      if (!response.ok || !data.url) throw new Error(data.error || "Checkout could not be started.");
      window.location.assign(data.url);
    } catch (value) {
      setError(value instanceof Error ? value.message : "Checkout could not be started.");
      setLoading(false);
    }
  }
  return (
    <span className="buyWrap">
      <button className={`button buttonPrimary ${className}`} disabled={loading || !available} onClick={startCheckout}>
        {available ? (loading ? "Opening secure checkout…" : "Buy NoBS PDF — £9.99") : "Desktop apps coming soon"} {available && <span aria-hidden="true">↗</span>}
      </button>
      {error && <small role="alert">{error}</small>}
    </span>
  );
}

function Header({ releases }: { releases: Releases }) {
  return (
    <header className="siteHeader shell">
      <Logo />
      <nav aria-label="Main navigation">
        <a href="#how">How it works</a><a href="#proof">Proof</a><a href="#pricing">Pricing</a>
      </nav>
      <BuyButton available={releases.macOS || releases.Windows} className="headerBuy" />
    </header>
  );
}

function SectionIntro({ kicker, title, children }: { kicker: string; title: string; children?: ReactNode }) {
  return <div className="sectionIntro"><p className="kicker">{kicker}</p><h2>{title}</h2>{children}</div>;
}

function Hero({ releases }: { releases: Releases }) {
  return (
    <section className="hero shell">
      <div className="heroCopy">
        <p className="editorial">Intelligent PDF Optimisation</p>
        <h1>Smaller PDFs.<br /><span>Without the bullshit.</span></h1>
        <p className="lede">Intelligent PDF optimisation that actually understands what’s inside your document.</p>
        <div className="heroActions"><BuyButton available={releases.macOS || releases.Windows} /><a className="button buttonText" href="#how">See how it works <span>↓</span></a></div>
        <p className="platforms">Available for {platformSummary(releases)} · macOS coming soon · Processes locally</p>
      </div>
      <div className="heroObject" aria-label="NoBS PDF application icon">
        <div className="iconField"><img src="/brand/nobs-app-icon.svg" alt="NoBS PDF" /></div>
        <span>One job. Done properly.</span>
      </div>
    </section>
  );
}

function Benchmark() {
  const b = content.benchmark;
  return (
    <section className="benchmark section" id="proof">
      <div className="shell">
        <SectionIntro kicker="The proof" title="A big PDF. Made sensible.">
          <p>One real document. One intelligent pass. No invented statistics.</p>
        </SectionIntro>
        <div className="resultCard">
          <div className="resultHero"><span>60 MB</span><i>→</i><strong>11.8 MB</strong></div>
          <div className="reduction"><b>{b.reduction}</b><span>smaller</span></div>
          <p>An 80.6% reduction — with excellent image quality, selectable text and preserved page geometry.</p>
          <dl className="benchmarkFacts">
            <div><dt>Input</dt><dd>{b.input}</dd></div><div><dt>Output</dt><dd>{b.output}</dd></div>
            <div><dt>Pages</dt><dd>{b.pages}</dd></div><div><dt>Raster objects</dt><dd>{b.rasterObjects}</dd></div>
            <div><dt>Placements composited</dt><dd>{b.composited}</dd></div><div><dt>Max. mean render error</dt><dd>{b.renderError}</dd></div>
            <div className="pass"><dt>Validation</dt><dd>✓ {b.validation}</dd></div>
          </dl>
        </div>
      </div>
    </section>
  );
}

function Problem() {
  const measures = ["Source resolution", "Physical placement", "Effective resolution", "Image dimensions", "Page geometry", "Raster usage", "Document structure"];
  return (
    <section className="problem section shell">
      <SectionIntro kicker="The problem" title="Most PDF compressors treat every image the same." />
      <div className="problemGrid">
        <div className="explanation"><p>You can place a 10,000px image into an InDesign document at 50mm wide. The PDF may contain thousands of pixels that will never actually be visible at the size they’re being used.</p><p><strong>NoBS understands the difference.</strong> It makes decisions based on how the artwork is actually being used.</p></div>
        <div className="measureList">{measures.map((item, i) => <span key={item}><b>{String(i + 1).padStart(2, "0")}</b>{item}<i>✓</i></span>)}</div>
      </div>
    </section>
  );
}

function HowItWorks() {
  return (
    <section className="how section" id="how"><div className="shell">
      <SectionIntro kicker="How it works" title="Three steps. No nonsense." />
      <div className="steps">{content.steps.map(([number, title, text]) => <article key={number}><span>{number}</span><div className={`stepGraphic graphic${number}`} aria-hidden="true"><i /><i /><i /></div><h3>{title}</h3><p>{text}</p></article>)}</div>
    </div></section>
  );
}

function Structure() {
  return (
    <section className="structure section shell">
      <SectionIntro kicker="The important difference" title="Not everything needs to become pixels."><p>Flatten the artwork that benefits from flattening. Keep the text you still need to select.</p></SectionIntro>
      <div className="layers">
        <article className="raster"><span>01</span><div className="pattern" /><h3>Raster artwork</h3><p>Resize and optimise heavy image data based on its actual use.</p><b>OPTIMISE</b></article>
        <article className="vector"><span>02</span><div className="vectorShape">◇</div><h3>Vector artwork</h3><p>Preserve precise, scalable artwork where the document supports it.</p><b>PRESERVE</b></article>
        <article className="textLayer"><span>03</span><div className="textShape">Aa</div><h3>Selectable text</h3><p>Keep text separate and selectable when that option is enabled.</p><b>KEEP SELECTABLE</b></article>
      </div>
    </section>
  );
}

function LocalProcessing() {
  return (
    <section className="local section"><div className="shell localGrid">
      <div><p className="kicker">Local processing</p><h2>Your PDF stays<br />on your machine.</h2><p>NoBS processes your files locally. Your documents don’t need to be uploaded to a server just to make them smaller.</p><strong>Local processing · Your PDF never leaves this device</strong></div>
      <div className="device" aria-hidden="true"><div className="deviceTop"><i /><i /><i /></div><div className="file"><span>PDF</span><b>annual-report.pdf</b><small>Processing locally…</small></div><div className="localLine"><span>YOUR MAC</span><i /><span>NO CLOUD</span></div></div>
    </div></section>
  );
}

function AppPreview({ releases }: { releases: Releases }) {
  return (
    <section className="appSection section shell">
      <SectionIntro kicker="The desktop app" title="A proper utility. Not another upload form."><p>A focused native desktop app for {platformSummary(releases)}.</p></SectionIntro>
      <div className="appWindow">
        <div className="titlebar"><span><i /><i /><i /></span><b>NoBS PDF</b></div>
        <div className="appBody"><Logo compact /><div className="appPanel">
          <div className="appFile"><b>PDF</b><span><strong>annual-report.pdf</strong><small>60 MB · 15 pages · 108 images</small></span><em>Change</em></div>
          <div className="appRule" /><label>DOCUMENT RESOLUTION <span>Same as source⌄</span></label>
          <div className="appOption"><span><b>FLATTEN PAGE ARTWORK</b><small>Combine page artwork into a single optimised raster layer.</small></span><i>✓</i></div>
          <div className="appLock">⌁ Page geometry, image placement and aspect ratios preserved <b>LOCKED</b></div>
          <div className="appEstimate"><span><small>ORIGINAL</small><b>60 MB</b></span><i>→</i><span><small>ESTIMATED</small><b>~11.8 MB</b></span><strong><small>EST. SAVING</small><b>~80.6%</b></strong></div>
          <button>Optimise PDF</button>
        </div></div>
      </div>
    </section>
  );
}

function Pricing({ releases }: { releases: Releases }) {
  const [price, setPrice] = useState("Loading…");
  const checkoutCancelled = new URLSearchParams(window.location.search).get("checkout") === "cancelled";
  useEffect(() => {
    void fetch("/api/config").then(async (response) => {
      if (!response.ok) { setPrice("Temporarily unavailable"); return; }
      const data = await response.json();
      if (typeof data.unitAmount !== "number" || typeof data.currency !== "string") return;
      setPrice(new Intl.NumberFormat(undefined, { style: "currency", currency: data.currency.toUpperCase() }).format(data.unitAmount / 100));
    }).catch(() => setPrice("Temporarily unavailable"));
  }, []);
  return (
    <section className="pricing section" id="pricing"><div className="shell pricingGrid">
      <SectionIntro kicker="Simple pricing" title="One payment. Yours forever."><p>No subscription. No renewal. No pricing maze.</p></SectionIntro>
      <div className="priceCard"><img src="/brand/nobs-icon-green.svg" alt="" /><span>NoBS PDF</span><strong>{price}</strong><p>One payment. Yours forever.</p>{checkoutCancelled && <div className="checkoutNotice">Checkout was cancelled. You haven’t been charged.</div>}<ul><li>No subscription</li><li>Up to 2 devices</li><li>Unlimited PDF optimisation</li><li>Files never leave your computer</li><li>{releases.Windows ? "Windows available" : "Windows coming soon"}</li><li>{releases.macOS ? "macOS available" : "macOS coming soon"}</li></ul><BuyButton available={releases.macOS || releases.Windows} /></div>
      <p className="pricingStatement">NO SUBSCRIPTION. NO UPLOAD. NO BULLSHIT.</p>
    </div></section>
  );
}

const baseFaqs = [
  ["What does NoBS actually do?", "It analyses raster objects inside a PDF and optimises them according to how they are placed and used, while preserving document structure wherever supported."],
  ["Does it change image quality?", "It intentionally changes raster resolution and encoding where appropriate. The goal is excellent visual quality at a dramatically smaller size—not lossless compression."],
  ["Does it preserve selectable text?", "Yes, selectable text can be kept separate when the preservation option is enabled."],
  ["Does it change image aspect ratios?", "NoBS preserves image aspect ratios during optimisation."],
  ["Does my PDF leave my computer?", "NoBS processes PDFs locally. Your document does not need to be uploaded to a server for optimisation."],
  ["Is this a subscription?", "No. NoBS PDF is a one-time £9.99 purchase. Your licence does not renew or expire."],
  ["Can I use it on multiple computers?", siteConfig.multiComputerPolicy],
] as const;

function FAQ({ releases }: { releases: Releases }) {
  const [open, setOpen] = useState<number | null>(0);
  const faqs = [...baseFaqs, ["Which platforms are available?", `NoBS PDF is available for ${platformSummary(releases)}.`] as const];
  return <section className="faq section shell"><SectionIntro kicker="Questions, answered" title="Frequently asked. Plainly answered." /><div className="faqList">{faqs.map(([q, a], i) => <div className="faqItem" key={q}><button aria-expanded={open === i} onClick={() => setOpen(open === i ? null : i)}><span>{q}</span><i>{open === i ? "−" : "+"}</i></button>{open === i && <p>{a}</p>}</div>)}</div></section>;
}

function FinalCTA({ releases }: { releases: Releases }) {
  return <section className="finalCta section shell"><img src="/brand/nobs-icon-warm.svg" alt="" /><p className="editorial">Intelligent PDF Optimisation</p><h2>No more 60 MB PDFs.<br /><span>Make them make sense.</span></h2><BuyButton available={releases.macOS || releases.Windows} /></section>;
}

function Footer() {
  return <footer className="shell"><Logo compact /><nav><a href="/privacy">Privacy</a><a href="/terms">Terms</a><a href="/refunds">Refunds</a><a href={`mailto:${siteConfig.supportEmail}`}>Support</a></nav><small>© {new Date().getFullYear()} NoBS PDF</small></footer>;
}

function DownloadPage() {
  const sessionId = new URLSearchParams(window.location.search).get("session_id") ?? "";
  const [state, setState] = useState<"loading" | "complete" | "error">("loading");
  const [purchase, setPurchase] = useState<{ email: string; licenceKey: string; releaseVersion: string; downloads: Releases } | null>(null);
  const [message, setMessage] = useState("Confirming your payment…");
  useEffect(() => {
    if (!sessionId) { setState("error"); setMessage("This download link is missing its secure purchase reference."); return; }
    let cancelled = false;
    let attempts = 0;
    async function checkPurchase() {
      try {
        const response = await fetch(`/api/purchases/${encodeURIComponent(sessionId)}`, { cache: "no-store" });
        const data = await response.json();
        if (cancelled) return;
        if (response.ok && data.status === "complete") { setPurchase(data.purchase); setState("complete"); return; }
        if (response.status === 202 && attempts++ < 12) { window.setTimeout(checkPurchase, 1500); return; }
        setState("error"); setMessage(data.error || "Payment is still being confirmed. Please refresh in a moment.");
      } catch { if (!cancelled) { setState("error"); setMessage("We couldn’t confirm the purchase. Please refresh or contact support."); } }
    }
    void checkPurchase();
    return () => { cancelled = true; };
  }, [sessionId]);

  if (state !== "complete" || !purchase) return <main className="download"><Logo /><div className="downloadCard"><span className="successMark">{state === "loading" ? "…" : "!"}</span><p className="kicker">PAYMENT CONFIRMATION</p><h1>{state === "loading" ? "Just a moment." : "We can’t show the download yet."}</h1><p>{message}</p><a className="backHome" href="/">← Back to NoBS PDF</a></div></main>;
  return <main className="download"><Logo /><div className="downloadCard"><span className="successMark">✓</span><p className="kicker">LICENCE READY</p><h1>You’re in.</h1><p>NoBS PDF is ready for <strong>{purchase.email}</strong>.</p><div className="licence"><small>YOUR LICENCE KEY</small><code>{purchase.licenceKey}</code></div><div>{purchase.downloads.Windows && <a className="button buttonPrimary" href={`/api/download/windows?session_id=${encodeURIComponent(sessionId)}`}>Download for Windows</a>}{purchase.downloads.macOS ? <a className="button buttonPrimary" href={`/api/download/mac?session_id=${encodeURIComponent(sessionId)}`}>Download for Mac</a> : <span className="button buttonText" aria-disabled="true">Mac — Coming soon</span>}</div><small>Your perpetual licence covers release {purchase.releaseVersion} on up to two devices.</small><a className="backHome" href="/">← Back to NoBS PDF</a></div></main>;
}

function Home() {
  const [releases, setReleases] = useState<Releases>(noReleases);
  useEffect(() => {
    void fetch("/api/config").then(async (response) => {
      if (!response.ok) return;
      const data = await response.json();
      if (typeof data.releases?.macOS === "boolean" && typeof data.releases?.Windows === "boolean") setReleases(data.releases);
    }).catch(() => undefined);
  }, []);
  return <><Header releases={releases} /><main><Hero releases={releases} /><Benchmark /><Problem /><HowItWorks /><Structure /><LocalProcessing /><AppPreview releases={releases} /><Pricing releases={releases} /><FAQ releases={releases} /><FinalCTA releases={releases} /></main><Footer /></>;
}

export default function App() {
  if (window.location.pathname === "/download" || window.location.pathname === "/success") return <DownloadPage />;
  return <Home />;
}
