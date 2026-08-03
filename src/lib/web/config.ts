/** The one place the sample workspace's API host is defined. */
export const HOSTED_MOCK_API = "https://api.mandalo.dev";
export const LOCAL_MOCK_API = "http://localhost:8787";

/**
 * Flip to "hosted" once api.mandalo.dev is live; the seeded workspace ships
 * both environments either way, so nothing else has to change.
 */
export type MockEnv = "local" | "hosted";
export const DEFAULT_MOCK_ENV = "hosted" as MockEnv;

export const ENV_LOCAL = "local";
export const ENV_HOSTED = "hosted";

export const DEFAULT_ENV_NAME =
  DEFAULT_MOCK_ENV === "hosted" ? ENV_HOSTED : ENV_LOCAL;

export const DOWNLOAD_URL = "https://github.com/De-Rus/mandalo/releases/latest";

export const BUILD_ID = "202608032340";
