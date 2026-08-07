import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  errorMessage,
  githubLogout,
  githubPollLogin,
  githubStartLogin,
  githubStatus,
} from "../lib/api";
import { currentHost } from "../lib/host";
import { toast } from "../store/toast";

export interface GithubSession {
  login: string | null;
  loading: boolean;
  signingIn: boolean;
  userCode: string | null;
  verifyUri: string | null;
  error: string | null;
  refresh: () => Promise<void>;
  signIn: () => Promise<void>;
  signOut: () => Promise<void>;
  clearError: () => void;
}

/** Device-flow session shared by Clone from GitHub and Connect remote. */
export function useGithubSession(): GithubSession {
  const [login, setLogin] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [signingIn, setSigningIn] = useState(false);
  const [userCode, setUserCode] = useState<string | null>(null);
  const [verifyUri, setVerifyUri] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pollRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearPoll = () => {
    if (pollRef.current) {
      clearTimeout(pollRef.current);
      pollRef.current = null;
    }
  };

  const refresh = useCallback(async () => {
    if (currentHost() !== "desktop") {
      setLogin(null);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const s = await githubStatus();
      setLogin(s.connected ? s.login : null);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
      setLogin(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    return clearPoll;
  }, [refresh]);

  const signIn = useCallback(async () => {
    setSigningIn(true);
    setError(null);
    try {
      const start = await githubStartLogin(false);
      setUserCode(start.userCode);
      setVerifyUri(start.verificationUri);
      try {
        await openUrl(start.verificationUri);
      } catch {
        /* user can open the link by hand */
      }
      const tick = async () => {
        try {
          const poll = await githubPollLogin(start.handle);
          if (poll.pending) {
            pollRef.current = setTimeout(() => void tick(), start.interval * 1000);
            return;
          }
          setLogin(poll.user?.login ?? null);
          setUserCode(null);
          setVerifyUri(null);
          setSigningIn(false);
          toast("success", `Signed in as ${poll.user?.login ?? "GitHub"}`);
        } catch (e) {
          setError(errorMessage(e));
          setSigningIn(false);
          setUserCode(null);
        }
      };
      pollRef.current = setTimeout(() => void tick(), start.interval * 1000);
    } catch (e) {
      setError(errorMessage(e));
      setSigningIn(false);
    }
  }, []);

  const signOut = useCallback(async () => {
    clearPoll();
    try {
      await githubLogout();
      setLogin(null);
      setUserCode(null);
      setVerifyUri(null);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, []);

  return {
    login,
    loading,
    signingIn,
    userCode,
    verifyUri,
    error,
    refresh,
    signIn,
    signOut,
    clearError: () => setError(null),
  };
}
