export const siteConfig = {
  supportEmail: "support@nobs-pdf.com",
  multiComputerPolicy: "Yes. One licence can be active on up to two devices at a time.",
} as const;

export const content = {
  benchmark: {
    input: "61,002,045 bytes",
    output: "11,835,505 bytes",
    reduction: "80.6%",
    pages: "15",
    rasterObjects: "108",
    composited: "88",
    renderError: "5.82 / 255",
    validation: "PASS",
  },
  steps: [
    ["01", "Analyse", "NoBS breaks the PDF down and understands what’s actually making it heavy."],
    ["02", "Optimise", "Images are resized and encoded according to how they’re being used — not using one blunt compression setting."],
    ["03", "Export", "Get a dramatically smaller PDF while preserving the document’s important structure."],
  ],
} as const;
