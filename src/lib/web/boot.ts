import { DEFAULT_ENV_NAME } from "./config";

const ENV_KEY = "mandalo.env.selected";

// The environment store reads this key while its module is evaluating, so the
// default has to be in place before anything imports it.
if (localStorage.getItem(ENV_KEY) === null)
  localStorage.setItem(ENV_KEY, DEFAULT_ENV_NAME);
