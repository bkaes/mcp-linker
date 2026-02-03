import { useMcpRefresh } from "@/contexts/McpRefreshContext";
import { useGithubReadmeJson } from "@/hooks/useGithubReadmeJson";
import { useClientPathStore } from "@/stores/clientPathStore";
import { useConfigFileStore } from "@/stores/configFileStore";
import { useTeamStore } from "@/stores/team";
import { invoke } from "@tauri-apps/api/core";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { useEffect, useRef, useState } from "react";
import { toast } from "sonner";
import { useServerConfig } from "../hooks/useServerConfig";
import { useServerTemplateSubmit } from "./useServerTemplateSubmit";

// Custom hook for ServerTemplateDialog logic
export function useServerTemplateLogic(
  isOpen: boolean,
  setIsDialogOpen: (open: boolean) => void,
) {
  const { selectedClient, selectedPath } = useClientPathStore();
  const { refreshServerList } = useMcpRefresh();
  const [githubUrl, setGithubUrl] = useState("");
  const { loading, error, fetchAllJsonBlocks } = useGithubReadmeJson();
  const { getTeamConfigPath } = useConfigFileStore();
  const { selectedTeamId } = useTeamStore();

  // Server config state and handlers
  const {
    serverName,
    setServerName,
    serverType,
    setServerType,
    config,
    setConfig,
    handleArgsChange,
    handleCommandChange,
    handleUrl,
    handleEnvChange,
    handletHeaderChange,
    envValues,
    setEnvValues,
    headerValues,
    setHeaderValues,
  } = useServerConfig(isOpen, selectedClient);

  // State for JSON textarea content
  const [jsonText, setJsonText] = useState<string>(
    JSON.stringify({ [serverName]: config }, null, 2),
  );

  // Track whether the config change originated from the JSON tab
  // so the useEffect doesn't overwrite what the user just typed.
  const jsonTabEditRef = useRef(false);

  // State for multiple JSON blocks from GitHub README
  // Each item: { obj: any, path: string[] }
  const [githubJsonBlocks, setGithubJsonBlocks] = useState<
    { serverName: string; obj: any }[]
  >([]);

  // Sync jsonText when config changes from the form tab or GitHub import.
  // Skip when the change came from the JSON tab (user is editing directly).
  useEffect(() => {
    if (jsonTabEditRef.current) {
      jsonTabEditRef.current = false;
      return;
    }
    setJsonText(JSON.stringify({ [serverName]: config }, null, 2));
  }, [config, serverName]);

  // Recursively find the actual server config (object with "command" or "url")
  // Returns the config and the best server name found along the way
  const findServerConfig = (
    obj: any,
    parentKey?: string,
    depth = 0,
  ): { name: string; config: any } | null => {
    if (!obj || typeof obj !== "object" || depth > 4) return null;

    // Found it: this object has "command" or "url"
    if ("command" in obj || "url" in obj) {
      return { name: parentKey || "", config: obj };
    }

    // Check mcpServers wrapper first
    if (obj.mcpServers && typeof obj.mcpServers === "object") {
      const firstKey = Object.keys(obj.mcpServers)[0];
      if (firstKey) {
        const result = findServerConfig(obj.mcpServers[firstKey], firstKey, depth + 1);
        if (result) return result;
      }
    }

    // Recurse into child keys
    for (const key of Object.keys(obj)) {
      if (key === "mcpServers") continue; // already checked
      const inner = obj[key];
      if (inner && typeof inner === "object") {
        const result = findServerConfig(inner, key, depth + 1);
        if (result) {
          if (!result.name) result.name = key;
          return result;
        }
      }
    }

    return null;
  };

  // Parse JSON and extract the server config + name.
  // Returns the found result or null.
  const parseJsonText = (
    text: string,
  ): { name: string; config: any } | null => {
    try {
      const parsed = JSON.parse(text);
      return findServerConfig(parsed) ?? null;
    } catch {
      return null;
    }
  };

  // Parse and set config from any JSON format users might paste
  const commonParse = (parsed: any) => {
    const found = findServerConfig(parsed);
    if (found) {
      if (found.name) setServerName(found.name);
      applyParsedConfig(found.config);
    } else {
      // Nothing recognizable found, set as-is and let submit validation catch it
      applyParsedConfig(parsed);
    }
  };

  // Apply a parsed server config object and sync related state
  const applyParsedConfig = (serverConfig: any) => {
    setConfig(serverConfig);
    if (serverConfig.env) setEnvValues(serverConfig.env);
    if (serverConfig.headers) setHeaderValues(serverConfig.headers);
    if ("command" in serverConfig) {
      setServerType("stdio");
    } else if (serverConfig.type === "sse" || serverConfig.type === "http") {
      setServerType(serverConfig.type);
    } else if ("url" in serverConfig) {
      setServerType("http");
    }
  };

  // Handle paste button click
  const handlePasteJson = async () => {
    try {
      const text = await readText();
      setJsonText(text);
      try {
        const parsed = JSON.parse(text);
        jsonTabEditRef.current = true;
        commonParse(parsed);
        toast.success("Config pasted from clipboard");
      } catch {
        toast.error("Clipboard content is not valid JSON");
      }
    } catch (e) {
      console.log(e);
      toast.error("Failed to read clipboard");
    }
  };

  // Handle textarea blur: try to parse and update config
  const handleJsonBlur = () => {
    try {
      const parsed = JSON.parse(jsonText);
      jsonTabEditRef.current = true;
      commonParse(parsed);
    } catch {
      toast.error("Invalid JSON format");
    }
  };

  // Use the custom submit hook to get both handlers
  // selectedPath may be null, convert to undefined for the hook
  const { handleSubmit: rawHandleSubmit } = useServerTemplateSubmit({
    serverName,
    setIsDialogOpen,
    config,
    serverType,
    selectedClient,
    selectedPath: selectedPath || undefined,
  });

  // Wrap handleSubmit to always parse the current jsonText first.
  // This ensures that even if the user typed JSON and clicked submit
  // without blurring, the latest JSON text is used — not stale state.
  const handleSubmit = async () => {
    const found = parseJsonText(jsonText);
    if (found) {
      // Pass the freshly-parsed config directly to the submit handler
      // so it doesn't rely on React state that may be one render behind.
      await rawHandleSubmit({
        config: found.config,
        serverName: found.name || serverName,
      });
    } else {
      // jsonText isn't valid JSON (user might be on the form tab).
      // Fall through to use the current React state.
      await rawHandleSubmit();
    }
  };

  const handleSubmitTeamLocal = async () => {
    try {
      // Also parse jsonText for team local submit
      const found = parseJsonText(jsonText);
      const effectiveConfig = found?.config ?? config;
      const effectiveName = found?.name || serverName;

      const teamConfigPath = getTeamConfigPath(selectedTeamId);
      await invoke("add_mcp_server", {
        clientName: "custom",
        path: teamConfigPath,
        serverName: effectiveName,
        serverConfig: effectiveConfig,
      });

      // Refresh team data automatically
      refreshServerList("custom", teamConfigPath);

      toast.success(`add to Team Local ok`);
      setIsDialogOpen(false);
    } catch (e: any) {
      console.error(e);
      toast.error(e?.message || "fail to add to Team Local");
    }
  };

  // Find all server config objects with 'command' or 'url' under mcpServers (max 3 levels)
  function findCommandOrUrlObjectsSimple(
    obj: any,
  ): { serverName: string; obj: any }[] {
    const results: { serverName: string; obj: any }[] = [];
    if (
      obj &&
      typeof obj === "object" &&
      obj.mcpServers &&
      typeof obj.mcpServers === "object"
    ) {
      for (const serverName of Object.keys(obj.mcpServers)) {
        const serverConfig = obj.mcpServers[serverName];
        if (
          serverConfig &&
          typeof serverConfig === "object" &&
          ("command" in serverConfig || "url" in serverConfig)
        ) {
          results.push({ serverName, obj: serverConfig });
        }
      }
    } else if (
      obj &&
      typeof obj === "object" &&
      ("command" in obj || "url" in obj)
    ) {
      // Fallback: root object is already a server config
      results.push({ serverName: "", obj });
    }
    return results;
  }

  // Handler for loading config from GitHub (multiple blocks)
  const handleLoadFromGithub = async () => {
    const blocks = await fetchAllJsonBlocks(githubUrl);
    if (blocks && blocks.length > 0) {
      let found: { serverName: string; obj: any }[] = [];
      for (let i = 0; i < blocks.length; ++i) {
        found = found.concat(findCommandOrUrlObjectsSimple(blocks[i]));
      }
      console.log("found", found);
      setGithubJsonBlocks(found);
      if (found.length > 0) {
        toast.success(`Loaded ${found.length} config object(s) from README`);
      } else {
        toast.error("No valid config objects with 'command' or 'url' found");
      }
    } else if (error) {
      toast.error(error);
    } else {
      toast.error("No JSON blocks found in README");
    }
  };

  // Expose all state and handlers needed by UI
  return {
    selectedClient,
    selectedPath,
    githubUrl,
    setGithubUrl,
    loading,
    error,
    fetchAllJsonBlocks,
    serverName,
    setServerName,
    serverType,
    setServerType,
    config,
    setConfig,
    handleArgsChange,
    handleCommandChange,
    handleUrl,
    handleEnvChange,
    handletHeaderChange,
    envValues,
    setEnvValues,
    headerValues,
    setHeaderValues,
    jsonText,
    setJsonText,
    githubJsonBlocks,
    setGithubJsonBlocks,
    handlePasteJson,
    handleJsonBlur,
    handleSubmit,
    handleSubmitTeamLocal,
    handleLoadFromGithub,
    setIsDialogOpen,
  };
}
