import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import type {
  AppError,
  DocumentSummary,
  Estimate,
  Result,
  Stage,
  LicenceStatus,
} from "./types";

const stages: Stage[] = [
  "analysing",
  "planning",
  "optimising",
  "rebuilding",
  "validating",
];
const labels: Record<Stage, string> = {
  analysing: "Analysing PDF…",
  planning: "Planning safe changes…",
  optimising: "Optimising images…",
  rebuilding: "Rebuilding PDF…",
  validating: "Validating output…",
};
const LICENCE_DUE_CHECK_MILLISECONDS = 60 * 60 * 1000;
const formatBytes = (value: number) =>
  new Intl.NumberFormat(undefined, {
    style: "unit",
    unit: "megabyte",
    unitDisplay: "short",
    maximumFractionDigits: 1,
  }).format(value / 1_000_000);

function MainApplication({ onManageLicence }: { onManageLicence: () => void }) {
  const [document, setDocument] = useState<DocumentSummary | null>(null);
  const [scale, setScale] = useState(100);
  const [estimate, setEstimate] = useState<Estimate | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [stage, setStage] = useState<Stage | null>(null);
  useEffect(() => {
    const unlisten = getCurrentWebviewWindow().onDragDropEvent((event) => {
      if (event.payload.type === "over") {
        setDragging(true);
      } else if (event.payload.type === "leave") {
        setDragging(false);
      } else {
        setDragging(false);
        const path = event.payload.paths[0];
        if (path) void loadPdf(path);
      }
    });
    const stageListener = listen<Stage>("optimisation-stage", (event) =>
      setStage(event.payload),
    );
    return () => {
      void unlisten.then((fn) => fn());
      void stageListener.then((fn) => fn());
    };
  }, []);
  useEffect(() => {
    if (!document || !document.resolution.has_raster_content) return;
    const timer = window.setTimeout(() => {
      setError(null);
      setLoading(true);
      setStage("planning");
      void invoke<Estimate>("estimate_pdf", { path: document.path, scalePercent: scale })
        .then(setEstimate)
        .catch((value) => setError(value as AppError))
        .finally(() => { setLoading(false); setStage(null); });
    }, 180);
    return () => window.clearTimeout(timer);
  }, [document, scale]);
  async function loadPdf(path: string) {
    setScale(100);
    setError(null);
    setResult(null);
    setEstimate(null);
    setLoading(true);
    setStage("analysing");
    try {
      const summary = await invoke<DocumentSummary>("inspect_pdf", { path });
      setDocument(summary);
      if (!summary.resolution.has_raster_content) setEstimate(null);
    } catch (value) {
      setDocument(null);
      setError(value as AppError);
    } finally {
      setLoading(false);
      setStage(null);
    }
  }
  async function browse() {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "PDF documents", extensions: ["pdf"] }],
    });
    if (typeof path === "string") await loadPdf(path);
  }
  async function optimise() {
    if (!document) return;
    const suffix = `${scale}pct`;
    const suggested = document.filename.replace(
      /\.pdf$/i,
      `_NoBS_${suffix}.pdf`,
    );
    const output = await save({
      defaultPath: suggested,
      filters: [{ name: "PDF document", extensions: ["pdf"] }],
    });
    if (!output) return;
    setError(null);
    setLoading(true);
    setStage("analysing");
    try {
      setResult(
        await invoke<Result>("optimise_pdf", {
          path: document.path,
          scalePercent: scale,
          outputPath: output,
        }),
      );
    } catch (value) {
      setError(value as AppError);
    } finally {
      setLoading(false);
      setStage(null);
    }
  }
  async function cancel() {
    await invoke("cancel_optimisation");
  }
  function reset() {
    setScale(100);
    setDocument(null);
    setEstimate(null);
    setResult(null);
    setError(null);
    setStage(null);
  }
  if (result) {
    const checks = [
          ["Text preserved", result.text_preserved],
          ["Vector artwork preserved", result.vectors_preserved],
          ["Aspect ratios preserved", result.aspect_ratios_preserved],
          ["Page geometry preserved", result.page_layout_preserved],
          ["PDF validated", result.validation_passed],
        ];
    return (
      <main>
        <Brand onManageLicence={onManageLicence} />
        <section className="success">
          <div className="successMark">✓</div>
          <p className="eyebrow">OPTIMISED · {result.scale_percent ?? 100}% SIZE</p>
          <h1>Your PDF is ready.</h1>
          <div className="resultGrid">
            <Metric
              label="Original"
              value={formatBytes(result.original_size_bytes)}
            />
            <span className="sizeArrow">→</span>
            <Metric
              label="Output"
              value={formatBytes(result.output_size_bytes)}
            />
            <div className="savingPayoff">
              <span>SMALLER</span>
              <strong>{result.saved_percent.toFixed(0)}%</strong>
            </div>
          </div>
          <div className="checks">
            {checks.map(([name, ok]) => (
              <span key={String(name)} className={ok ? "ok" : "bad"}>
                {ok ? "✓" : "×"} {name}
              </span>
            ))}
          </div>
          <div className="actions">
            <button
              className="primary"
              onClick={() => openPath(result.output_path)}
            >
              Open PDF
            </button>
            <button onClick={() => revealItemInDir(result.output_path)}>
              Show in folder
            </button>
          </div>
          <button className="link" onClick={reset}>
            Optimise another
          </button>
        </section>
      </main>
    );
  }
  return (
    <main>
      <Brand onManageLicence={onManageLicence} />
      <section className="panel">
        {!document ? (
          <button
            className={`dropzone ${dragging ? "dragging" : ""}`}
            onClick={browse}
          >
            <span className="fileIcon">PDF</span>
            <strong>{dragging ? "Drop to inspect" : "Drop PDF here"}</strong>
            <small>or click to browse</small>
          </button>
        ) : (
          <>
            <div className="document">
              <div className="pdfBadge">PDF</div>
              <div>
                <strong>{document.filename}</strong>
                <span>
                  {formatBytes(document.size_bytes)} · {document.page_count}{" "}
                  pages · {document.image_count} images
                </span>
              </div>
              <button className="quiet" onClick={reset}>
                Change
              </button>
            </div>
            <div className="rule" />
            <SizeControl document={document} scale={scale} onChange={setScale} />
            <div className="lock">⌁ <span>Text, graphics and page layout stay sharp and unchanged</span><b>LOCKED</b></div>
            {estimate && (
              <div className="estimate">
                <Metric
                  label="Original"
                  value={formatBytes(estimate.original_size_bytes)}
                />
                <span className="sizeArrow">→</span>
                <Metric
                  label="Estimated"
                  value={
                    estimate.estimated_output_size_bytes != null
                      ? `~${formatBytes(estimate.estimated_output_size_bytes)}`
                      : "Calculated after export"
                  }
                />
                <div className="savingPayoff">
                  <span>EST. SAVING</span>
                  <strong>
                    {estimate.estimated_saving_percent != null
                      ? `~${estimate.estimated_saving_percent.toFixed(0)}%`
                      : "—"}
                  </strong>
                </div>
              </div>
            )}
            <button
              className="primary optimise"
              disabled={loading || (document.resolution.has_raster_content && !estimate)}
              onClick={optimise}
            >
              Optimise
            </button>
          </>
        )}
      </section>
      {loading && stage && (
        <div className="progress">
          <div className="spinner" />
          <div>
            <strong>{labels[stage]}</strong>
            <div className="stageDots">
              {stages.map((item) => (
                <i
                  key={item}
                  className={
                    stages.indexOf(item) <= stages.indexOf(stage)
                      ? "active"
                      : ""
                  }
                />
              ))}
            </div>
          </div>
          <button className="quiet" onClick={cancel}>
            Cancel
          </button>
        </div>
      )}
      {error && (
        <div className="error">
          <strong>
            {error.code === "validation_failed"
              ? "Optimisation failed validation"
              : error.message}
          </strong>
          <span>
            {error.code === "validation_failed"
              ? "No output was created because it did not pass NoBS PDF safety checks."
              : error.message}
          </span>
          {error.detail && (
            <details>
              <summary>Technical detail</summary>
              {error.detail}
            </details>
          )}
          <button onClick={() => setError(null)}>Back</button>
        </div>
      )}
      <footer>Local processing · Your PDF never leaves this device</footer>
    </main>
  );
}
function Brand({ onManageLicence }: { onManageLicence?: () => void }) {
  return (
    <header>
      <img
        className="brandMark"
        src="/brand/nobs-icon-monochrome.svg"
        alt=""
        aria-hidden="true"
      />
      <div>
        <h2>NoBS PDF</h2>
        <p>Intelligent PDF Optimisation</p>
      </div>
      {onManageLicence && <button className="licenceButton" onClick={onManageLicence} aria-label="Manage licence">Licence</button>}
    </header>
  );
}
function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function scaledDimensions(dimensions: [number, number], scale: number) {
  return dimensions.map((value) => Math.max(1, Math.round(value * scale / 100))) as [number, number];
}

function SizeControl({ document, scale, onChange }: { document: DocumentSummary; scale: number; onChange: (scale: number) => void }) {
  const resolution = document.resolution;
  const dimensions = resolution.representative_100_percent;
  const scaled = dimensions ? scaledDimensions(dimensions, scale) : null;
  const vectorOnly = !resolution.has_raster_content;
  const title = vectorOnly ? "Native vector document" : resolution.adaptive && !dimensions
    ? "Adaptive raster document" : resolution.mixed_page_sizes ? "Mixed page sizes"
    : dimensions ? `${dimensions[0]} × ${dimensions[1]}` : "Adaptive raster document";
  return <section className={`sizeControl ${vectorOnly ? "disabled" : ""}`}>
    <div className="sizeHeading"><div><span>DOCUMENT SIZE</span><strong>{title}</strong></div>
      {!vectorOnly && resolution.mixed_page_sizes && dimensions && <small>Up to {dimensions[0]} × {dimensions[1]} at 100%</small>}
      {vectorOnly && <small>No raster content needs resizing.</small>}
    </div>
    <div className="sliderLabels"><span>10%</span><b>SIZE</b><span>100%</span></div>
    <input type="range" min="10" max="100" step="1" value={scale} disabled={vectorOnly} aria-label="Document size percentage" onChange={(event) => onChange(Number(event.target.value))} />
    <div className="scaleResult"><strong>{vectorOnly ? "100% · Native vector" : `${scale}%${scale === 100 ? " · Original" : ""}`}</strong><span>{vectorOnly ? "Text and graphics remain native" : scaled ? `${scaled[0]} × ${scaled[1]}` : `${scale}% raster limit`}</span></div>
    {!vectorOnly && <p className="sizeNote">{scale === 100 ? "Original document size · oversized images are still optimised." : "Only images are reduced · text and graphics stay sharp."}</p>}
  </section>;
}

function formatLicenceInput(value: string) {
  let compact = value.toUpperCase().replace(/[^A-Z0-9]/g, "");
  if (compact.startsWith("NOBS")) compact = compact.slice(4);
  compact = compact.slice(0, 16);
  return `NOBS${compact ? `-${compact.match(/.{1,4}/g)?.join("-")}` : "-"}`;
}

function ActivationScreen({ status, onActivated }: { status: LicenceStatus; onActivated: (status: LicenceStatus) => void }) {
  const [licenceKey, setLicenceKey] = useState(status.licenceKey ?? "NOBS-");
  const [submitting, setSubmitting] = useState(false);
  const [response, setResponse] = useState(status);
  async function activate() {
    if (submitting) return;
    setSubmitting(true);
    const next = await invoke<LicenceStatus>("activate_licence", { licenceKey });
    setResponse(next);
    setSubmitting(false);
    if (next.state === "ACTIVE") onActivated(next);
  }
  return <main className="activationPage">
    <Brand />
    <section className="activationCard">
      <p className="eyebrow">FIRST LAUNCH</p>
      <h1>Activate NoBS PDF</h1>
      <p>Enter your licence key to continue.</p>
      <label className="licenceField">
        <span>LICENCE KEY</span>
        <input autoFocus autoCapitalize="characters" autoCorrect="off" spellCheck={false} value={licenceKey} onChange={(event) => setLicenceKey(formatLicenceInput(event.target.value))} onKeyDown={(event) => { if (event.key === "Enter") void activate(); }} placeholder="NOBS-____-____-____-____" />
      </label>
      {response.state !== "NOT_ACTIVATED" && response.message && <div className={`licenceMessage ${response.state === "NETWORK_ERROR" ? "warningMessage" : "errorMessage"}`} role="alert">{response.message}</div>}
      <button className="primary activationSubmit" disabled={submitting || licenceKey.length !== 24} onClick={() => void activate()}>{submitting ? "Verifying…" : "Activate"}</button>
      <small>Already activated on this device? Restart NoBS PDF or contact support if the activation is not detected.</small>
    </section>
  </main>;
}

function LicenceSettings({ status, onClose, onDeactivated }: { status: LicenceStatus; onClose: () => void; onDeactivated: (status: LicenceStatus) => void }) {
  const [submitting, setSubmitting] = useState(false);
  const [message, setMessage] = useState("");
  async function deactivate() {
    if (submitting) return;
    setSubmitting(true);
    const next = await invoke<LicenceStatus>("deactivate_licence");
    setSubmitting(false);
    if (next.state === "NOT_ACTIVATED") onDeactivated(next); else setMessage(next.message ?? "This device could not be deactivated.");
  }
  return <main className="activationPage"><Brand /><section className="activationCard licenceSettings">
    <p className="eyebrow">SETTINGS · LICENCE</p><h1>Licence</h1>
    <dl><div><dt>Licence</dt><dd>{status.licenceKey}</dd></div><div><dt>Status</dt><dd className="activeStatus">● Active</dd></div><div><dt>Device</dt><dd>{status.deviceName}</dd></div></dl>
    {message && <div className="licenceMessage errorMessage" role="alert">{message}</div>}
    <div className="licenceActions"><button className="primary" disabled={submitting} onClick={() => void deactivate()}>{submitting ? "Deactivating…" : "Deactivate"}</button><button onClick={onClose}>Done</button></div>
    <small>Deactivating this device frees an activation slot. An internet connection is required.</small>
  </section></main>;
}

export default function App() {
  const [licence, setLicence] = useState<LicenceStatus | null>(null);
  const [settings, setSettings] = useState(false);
  useEffect(() => {
    let active = true;
    const checkIfDue = () => {
      void invoke<LicenceStatus>("revalidate_licence").then((remote) => {
        if (
          active &&
          (remote.state === "ACTIVE" ||
            remote.state === "REVOKED" ||
            remote.state === "EXPIRED" ||
            (remote.state === "INVALID" && !remote.locallyActivated))
        ) {
          setLicence(remote);
        }
      });
    };
    void invoke<LicenceStatus>("get_licence_status").then((local) => {
      if (!active) return;
      setLicence(local);
      if (local.state === "ACTIVE" || local.state === "EXPIRED") {
        checkIfDue();
      }
    });
    const interval = window.setInterval(checkIfDue, LICENCE_DUE_CHECK_MILLISECONDS);
    return () => {
      active = false;
      window.clearInterval(interval);
    };
  }, []);
  if (!licence) return <main className="activationPage"><Brand /><div className="licenceLoading">Checking activation…</div></main>;
  const usable = licence.state === "ACTIVE" || (licence.state === "NETWORK_ERROR" && licence.locallyActivated);
  if (!usable) return <ActivationScreen status={licence} onActivated={setLicence} />;
  if (settings) return <LicenceSettings status={licence} onClose={() => setSettings(false)} onDeactivated={(next) => { setLicence(next); setSettings(false); }} />;
  return <MainApplication onManageLicence={() => setSettings(true)} />;
}
