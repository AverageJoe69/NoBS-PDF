export type DocumentSummary = {
  path: string;
  filename: string;
  size_bytes: number;
  page_count: number;
  image_count: number;
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
  | "ACTIVATION_LIMIT_REACHED"
  | "NETWORK_ERROR";

export type LicenceStatus = {
  state: LicenceState;
  message?: string;
  licenceKey?: string;
  deviceName: string;
  locallyActivated: boolean;
};
