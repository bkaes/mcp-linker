import { useMcpRefresh } from "@/contexts/McpRefreshContext";
import { useCCProjectStore } from "@/stores/ccProject";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { transformConfig } from "../utils/transformConfig";

// Custom hook to provide submit handlers for server template dialog
export function useServerTemplateSubmit({
  serverName,
  setIsDialogOpen,
  config,
  serverType,
  selectedClient,
  selectedPath,
}: {
  serverName: string;
  setIsDialogOpen: (open: boolean) => void;
  config: any;
  serverType: string;
  selectedClient: string;
  selectedPath: string | undefined;
}) {
  const { refreshServerList } = useMcpRefresh();
  const { selectedProject } = useCCProjectStore();

  // Handle submit (add to local config)
  // Accepts optional overrides so callers can pass freshly-parsed values
  // (e.g. from the JSON tab) without relying on stale React state.
  const handleSubmit = async (overrides?: {
    config?: any;
    serverName?: string;
  }) => {
    const effectiveConfig = overrides?.config ?? config;
    const effectiveName = overrides?.serverName || serverName;

    try {
      if (selectedClient === "claude_code") {
        // Handle Claude Code separately - don't use transformConfig
        if (!selectedProject) {
          toast.error("Please select a Claude Code project in header");
          return;
        }
        const req: any = { name: effectiveName };
        if ((effectiveConfig as any).command) {
          req.type = "stdio";
          req.command = (effectiveConfig as any).command;
          req.args = (effectiveConfig as any).args || [];
          if ((effectiveConfig as any).env)
            req.env = (effectiveConfig as any).env;
        } else if ((effectiveConfig as any).url) {
          req.type = (effectiveConfig as any).type || "http";
          req.url = (effectiveConfig as any).url;
        } else {
          throw new Error("Unsupported config for Claude Code");
        }
        await invoke("claude_mcp_add", {
          request: req,
          workingDir: selectedProject,
        });
      } else {
        const finalConfig = transformConfig(serverType, effectiveConfig);
        await invoke("add_mcp_server", {
          clientName: selectedClient,
          path: selectedPath || undefined,
          serverName: effectiveName,
          serverConfig: finalConfig,
        });
      }

      // Refresh the server list automatically
      refreshServerList(selectedClient, selectedPath);

      toast.success("Configuration updated successfully");
      setIsDialogOpen(false);
    } catch (error) {
      console.error(`Error updating config: ${error}`);
      const message = error instanceof Error ? error.message : String(error);
      toast.error(`Failed to update configuration: ${message}`);
    }
  };

  return { handleSubmit };
}
