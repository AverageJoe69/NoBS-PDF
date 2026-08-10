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
  optimising: "Optimising raster artwork…",
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
  const [resolution, setResolution] = useState("source");
  const [flatten, setFlatten] = useState(false);
  const [preserveText, setPreserveText] = useState(true);
  const [estimate, setEstimate] = useState<Estimate | null>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [stage, setStage] = useState<Stage | null>(null);
  const profile = flatten
    ? `${preserveText ? "flatten_text" : "flatten"}:${resolution}`
    : resolution;
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
  async function loadPdf(path: string) {
    setResolution("source");
    setFlatten(false);
    setPreserveText(true);
    setError(null);
    setResult(null);
    setEstimate(null);
    setLoading(true);
    setStage("analysing");
    try {
      const summary = await invoke<DocumentSummary>("inspect_pdf", { path });
      setDocument(summary);
      setStage("planning");
      setEstimate(
        await invoke<Estimate>("estimate_pdf", { path, profile: "source" }),
      );
    } catch (value) {
      setDocument(null);
      setError(value as AppError);
    } finally {
      setLoading(false);
      setStage(null);
    }
  }
  async function changeOptions(
    nextResolution: string,
    nextFlatten: boolean,
    nextPreserveText = preserveText,
  ) {
    setResolution(nextResolution);
    setFlatten(nextFlatten);
    if (!document) return;
    setError(null);
    setEstimate(null);
    setLoading(true);
    setStage("planning");
    try {
      setEstimate(
        await invoke<Estimate>("estimate_pdf", {
          path: document.path,
          profile: nextFlatten
            ? `${nextPreserveText ? "flatten_text" : "flatten"}:${nextResolution}`
            : nextResolution,
        }),
      );
    } catch (value) {
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
    const suffix = `${flatten ? "Flattened_" : ""}${resolution === "source" ? "Source" : resolution}`;
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
          profile,
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
    setResolution("source");
    setFlatten(false);
    setPreserveText(true);
    setDocument(null);
    setEstimate(null);
    setResult(null);
    setError(null);
    setStage(null);
  }
  if (result) {
    const flattened = result.mode.startsWith("flatten");
    const textPreserved = result.mode === "flatten_text";
    const checks = flattened
      ? [
          ["Raster artwork flattened", true],
          [
            textPreserved
              ? "Selectable text preserved"
              : "Pages intentionally rasterised",
            textPreserved ? result.text_preserved : true,
          ],
          ["Aspect ratios preserved", result.aspect_ratios_preserved],
          ["Page geometry preserved", result.page_layout_preserved],
          ["PDF validated", result.validation_passed],
        ]
      : [
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
          <p className="eyebrow">{flattened ? "FLATTENED" : "OPTIMISED"}</p>
          <h1>Your PDF is ready.</h1>
          {flattened && !textPreserved && (
            <div className="warning compact">
              <strong>Full-page raster copy</strong>
              <span>
                Text is no longer selectable and vectors are now pixels.
              </span>
            </div>
          )}
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
            <label>
              DOCUMENT RESOLUTION
              <select
                value={resolution}
                onChange={(event) =>
                  void changeOptions(event.target.value, flatten)
                }
              >
                <option value="source">Same as source</option>
                <option value="720p">720p</option>
                <option value="1080p">1080p</option>
                <option value="1440p">1440p</option>
                <option value="4k">4K</option>
              </select>
            </label>
            <div className="optionRow">
              <div>
                <strong>FLATTEN PAGE ARTWORK</strong>
                <span>
                  Combine page artwork into a single optimised raster layer.
                </span>
              </div>
              <input
                type="checkbox"
                checked={flatten}
                onChange={(event) =>
                  void changeOptions(resolution, event.target.checked)
                }
              />
            </div>
            {flatten && (
              <div className="optionRow">
                <div>
                  <strong>Preserve selectable text</strong>
                  <span>
                    Keep text separate and selectable above the raster layer.
                  </span>
                </div>
                <input
                  type="checkbox"
                  checked={preserveText}
                  onChange={(event) => {
                    setPreserveText(event.target.checked);
                    void changeOptions(resolution, true, event.target.checked);
                  }}
                />
              </div>
            )}
            {flatten ? (
              <div className="warning">
                <strong>Rasterisation is destructive to vector artwork</strong>
                <span>
                  Vector graphics will become pixels. Text can remain selectable
                  if enabled.
                </span>
              </div>
            ) : (
              <div className="lock">
                ⌁{" "}
                <span>
                  Page geometry, image placement and aspect ratios preserved
                </span>
                <b>LOCKED</b>
              </div>
            )}
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
            {estimate && !flatten && (
              <AnalysisSummary estimate={estimate} profile={profile} />
            )}
            <button
              className="primary optimise"
              disabled={loading || !estimate}
              onClick={optimise}
            >
              {flatten ? "Flatten & optimise" : "Optimise PDF"}
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

function AnalysisSummary({
  estimate,
  profile,
}: {
  estimate: Estimate;
  profile: string;
}) {
  const originalResolution = profile === "source";
  return (
    <section className="analysisSummary">
      <div className="summaryHead">
        <div>
          <span className="eyebrow">INITIAL SUMMARY</span>
          <strong>
            {estimate.bloated_images.length}{" "}
            {originalResolution ? "oversized" : "bloated"}{" "}
            {estimate.bloated_images.length === 1 ? "image" : "images"}
          </strong>
        </div>
        <b>
          {originalResolution
            ? "Matched to placed document pixels"
            : `Document target: ${estimate.document_long_dimension_px}px`}
        </b>
      </div>
      {estimate.bloated_images.length ? (
        <div className="imageList">
          {estimate.bloated_images.map((image) => (
            <div className="imageRow" key={image.object_id}>
              <div className="imageIdentity">
                <strong>{image.object_id}</strong>
                <span>{formatBytes(image.original_bytes)}</span>
              </div>
              <div>
                <small>FILE RESOLUTION</small>
                <b>
                  {image.file_pixels[0]} × {image.file_pixels[1]} px
                </b>
              </div>
              <div className="arrow">→</div>
              <div>
                <small>
                  {originalResolution
                    ? "PIXELS USED IN PDF"
                    : "DOCUMENT PIXELS"}
                </small>
                <b>
                  {image.document_pixels[0]} × {image.document_pixels[1]} px
                </b>
              </div>
              <em>~{formatBytes(image.estimated_saving_bytes)} saved</em>
            </div>
          ))}
        </div>
      ) : (
        <p className="allGood">
          ✓ No safely{" "}
          {originalResolution
            ? "placement-oversized"
            : "downsampleable bloated"}{" "}
          images were found.
        </p>
      )}
    </section>
  );
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
          (remote.state === "REVOKED" ||
            (remote.state === "INVALID" && !remote.locallyActivated))
        ) {
          setLicence(remote);
        }
      });
    };
    void invoke<LicenceStatus>("get_licence_status").then((local) => {
      if (!active) return;
      setLicence(local);
      if (local.state === "ACTIVE") {
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
