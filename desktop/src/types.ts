export type DocumentSummary = {
  path: string;
  filename: string;
  size_bytes: number;
  page_count: number;
  image_count: number;
  resolution: DocumentResolution;
};
export type PageClassification = "digital" | "physical" | "vector_only" | "ambiguous";
export type PageRasterBudget = {
  page_number: number;
  classification: PageClassification;
  budget_100_percent: [number, number] | null;
  display_dimensions: boolean;
  confidence: boolean;
  reason: string;
};
export type DocumentResolution = {
  pages: PageRasterBudget[];
  has_raster_content: boolean;
  mixed_page_sizes: boolean;
  representative_100_percent: [number, number] | null;
  adaptive: boolean;
};
export type BloatedImage = {
  object_id: string;
  file_pixels: [number, number];
  document_pixels: [number, number];
  original_bytes: number;
  estimated_saving_bytes: number;
};
export type Estimate = {
  original_size_bytes: number;
  estimated_output_size_bytes: number | null;
  estimated_saving_bytes: number | null;
  estimated_saving_percent: number | null;
  candidate_images: number;
  skipped_images: number;
  profile: string;
  document_long_dimension_px: number | null;
  bloated_images: BloatedImage[];
  scale_percent: number | null;
  page_budgets: PageRasterBudget[];
};
export type Result = {
  mode: string;
  output_path: string;
  original_size_bytes: number;
  output_size_bytes: number;
  saved_bytes: number;
  saved_percent: number;
  images_optimised: number;
  images_skipped: number;
  validation_passed: boolean;
  page_layout_preserved: boolean;
  text_preserved: boolean;
  vectors_preserved: boolean;
  aspect_ratios_preserved: boolean;
  image_placement_preserved: boolean;
  scale_percent: number | null;
  page_budgets: PageRasterBudget[];
};
export type AppError = { code: string; message: string; detail?: string };
export type Stage =
  | "analysing"
  | "planning"
  | "optimising"
  | "rebuilding"
  | "validating";

export type LicenceState =
  | "NOT_ACTIVATED"
  | "ACTIVE"
  | "INVALID"
  | "REVOKED"
  | "EXPIRED"
  | "ACTIVATION_LIMIT_REACHED"
  | "NETWORK_ERROR";

export type LicenceStatus = {
  state: LicenceState;
  message?: string;
  licenceKey?: string;
  deviceName: string;
  locallyActivated: boolean;
};

export type UpdateStatus = {
  currentVersion: string;
  latestVersion: string;
  updateAvailable: boolean;
  downloadPage: string;
};
