import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import * as api from "@/lib/tauri-commands";

// ============ Tray snapshot (also drives the real 24h activity curve) ============

export function useTraySnapshot() {
  return useQuery({
    queryKey: ["tray_snapshot"],
    queryFn: api.getTraySnapshot,
    refetchInterval: 30_000,
    staleTime: 10_000,
  });
}

// ============ Providers ============

export function useProviders() {
  return useQuery({
    queryKey: ["providers"],
    queryFn: api.listProviders,
  });
}

export function useCreateProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (provider: api.Provider) => api.createProvider(provider),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["providers"] }),
  });
}

export function useUpdateProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (provider: api.Provider) => api.updateProvider(provider),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["providers"] }),
  });
}

export function useDeleteProvider() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteProvider(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["providers"] }),
  });
}

export function useTestProviderConnection() {
  return useMutation({
    mutationFn: (providerId: string) => api.testProviderConnection(providerId),
  });
}

export function useTestAccountConnection() {
  return useMutation({
    mutationFn: (accountId: string) => api.testAccountConnection(accountId),
  });
}

// ============ Accounts ============

export function useAccounts() {
  return useQuery({
    queryKey: ["accounts"],
    queryFn: api.listAccounts,
  });
}

export function useCreateAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (account: api.Account) => api.createAccount(account),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useUpdateAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (account: api.Account) => api.updateAccount(account),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

function invalidateAccountDependencies(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: ["accounts"] });
  qc.invalidateQueries({ queryKey: ["providers"] });
  qc.invalidateQueries({ queryKey: ["groups"] });
  qc.invalidateQueries({ queryKey: ["group_accounts"] });
  qc.invalidateQueries({ queryKey: ["group_available_models"] });
  qc.invalidateQueries({ queryKey: ["group_models"] });
  qc.invalidateQueries({ queryKey: ["group_dashboard"] });
  qc.invalidateQueries({ queryKey: ["route_topology"] });
}

export function useDeleteAccount() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteAccount(id),
    onSuccess: () => invalidateAccountDependencies(qc),
  });
}

export function useBatchDeleteAccounts() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (ids: string[]) => api.batchDeleteAccounts(ids),
    onSuccess: () => invalidateAccountDependencies(qc),
  });
}

export function useBatchUpdateAccounts() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ ids, status }: { ids: string[]; status: string }) =>
      api.batchUpdateAccounts(ids, status),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useAccountRequestCounts(days?: number) {
  return useQuery({
    queryKey: ["account_request_counts", days],
    queryFn: () => api.getAccountRequestCounts(days),
    staleTime: 60_000,
  });
}

export function usePreviewImport() {
  return useMutation({
    mutationFn: (request: api.ImportSourceRequest) => api.previewImportSource(request),
  });
}

export function useExecuteImport() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ request, options }: { request: api.ImportSourceRequest; options: api.ImportOptions }) =>
      api.executeImport(request, options),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useFetchUpstreamModels() {
  return useMutation({
    mutationFn: ({
      baseUrl,
      apiKey,
      protocol,
    }: {
      baseUrl: string;
      apiKey?: string;
      protocol?: string;
    }) => api.fetchUpstreamModels(baseUrl, apiKey, protocol),
  });
}

function invalidateModelResourceQueries(qc: ReturnType<typeof useQueryClient>) {
  qc.invalidateQueries({ queryKey: ["accounts"] });
  qc.invalidateQueries({ queryKey: ["providers"] });
  qc.invalidateQueries({ queryKey: ["group_available_models"] });
  qc.invalidateQueries({ queryKey: ["group_models"] });
  qc.invalidateQueries({ queryKey: ["group_dashboard"] });
  qc.invalidateQueries({ queryKey: ["route_topology"] });
}

export function useRefreshAccountModels() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.refreshAccountModels,
    onSuccess: () => invalidateModelResourceQueries(qc),
  });
}

export function useBatchRefreshAccountModels() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.batchRefreshAccountModels,
    onSuccess: () => invalidateModelResourceQueries(qc),
  });
}

export function usePreviewAndCheckImport() {
  return useMutation({
    mutationFn: ({ request, options }: { request: api.ImportSourceRequest; options: api.ImportOptions }) =>
      api.previewAndCheckImport(request, options),
  });
}

/** Scan well-known Cockpit/Codex config locations. */
export function useScanAgentConfigs() {
  return useMutation({
    mutationFn: () => api.scanAgentConfigs(),
  });
}

// ============ Health ============

export function useCheckAccountHealth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountId: string) => api.checkAccountHealth(accountId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useBatchCheckHealth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accountIds?: string[]) => api.batchCheckHealth(accountIds),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useStartOAuthLogin() {
  return useMutation({
    mutationFn: ({ providerId, emailHint, note }: { providerId: string; emailHint?: string; note?: string }) =>
      api.startOAuthLogin(providerId, emailHint, note),
  });
}

export function useCompleteOAuthLogin() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ loginId, callbackUrl }: { loginId: string; callbackUrl?: string }) =>
      api.completeOAuthLogin(loginId, callbackUrl),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useCancelOAuthLogin() {
  return useMutation({ mutationFn: api.cancelOAuthLogin });
}

export function useRefreshAccountToken() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.refreshAccountToken,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useBatchRefreshTokens() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.batchRefreshTokens,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useRefreshAccountQuota() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.refreshAccountQuota,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useBatchRefreshQuotas() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.batchRefreshQuotas,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

export function useCleanupExpired() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => api.cleanupExpired(),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["accounts"] }),
  });
}

// ============ Groups ============

export function useGroups() {
  return useQuery({
    queryKey: ["groups"],
    queryFn: api.listGroups,
  });
}

export function useCreateGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (group: api.AgentGroup) => api.createGroup(group),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["groups"] });
      qc.invalidateQueries({ queryKey: ["client_keys"] });
    },
  });
}

export function useEnsureGroupClientKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.ensureGroupClientKey,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["client_keys"] }),
  });
}

export function useRotateGroupClientKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.rotateGroupClientKey,
    onSuccess: () => qc.invalidateQueries({ queryKey: ["client_keys"] }),
  });
}

export function useUpdateGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (group: api.AgentGroup) => api.updateGroup(group),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["groups"] }),
  });
}

export function useDeleteGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteGroup(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["groups"] });
      qc.invalidateQueries({ queryKey: ["client_keys"] });
    },
  });
}

export function useGroupAccounts(groupId: string | null) {
  return useQuery({
    queryKey: ["group_accounts", groupId],
    queryFn: () => api.getGroupAccounts(groupId!),
    enabled: !!groupId,
  });
}

export function useRemoveAccountFromGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ groupId, accountId }: { groupId: string; accountId: string }) =>
      api.removeAccountFromGroup(groupId, accountId),
    onSuccess: (_, { groupId }) => qc.invalidateQueries({ queryKey: ["group_accounts", groupId] }),
  });
}

export function useAddAccountToGroup() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ groupId, accountId, weight }: { groupId: string; accountId: string; weight?: number }) =>
      api.addAccountToGroup(groupId, accountId, weight),
    onSuccess: (_, { groupId }) => qc.invalidateQueries({ queryKey: ["group_accounts", groupId] }),
  });
}

export function useGroupModelResources(groupId: string | null) {
  return useQuery({
    queryKey: ["group_models", groupId],
    queryFn: () => api.getGroupModelResources(groupId!),
    enabled: !!groupId,
  });
}

function invalidateGroupResources(qc: ReturnType<typeof useQueryClient>, groupId: string) {
  qc.invalidateQueries({ queryKey: ["group_models", groupId] });
  qc.invalidateQueries({ queryKey: ["group_available_models", groupId] });
  qc.invalidateQueries({ queryKey: ["group_dashboard", groupId] });
}

export function useAvailableGroupModelResources(groupId: string | null) {
  return useQuery({
    queryKey: ["group_available_models", groupId],
    queryFn: () => api.listAvailableGroupModelResources(groupId!),
    enabled: !!groupId,
  });
}

export function useAddGroupModelResources() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ groupId, resources }: { groupId: string; resources: api.GroupModelResource[] }) =>
      api.addGroupModelResources(groupId, resources),
    onSuccess: (_, { groupId }) => invalidateGroupResources(qc, groupId),
  });
}

export function useSetGroupModelAccountIds() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ groupId, providerId, model, accountIds }: {
      groupId: string;
      providerId: string;
      model: string;
      accountIds: string[];
    }) => api.setGroupModelAccountIds(groupId, providerId, model, accountIds),
    onSuccess: (_, { groupId }) => invalidateGroupResources(qc, groupId),
  });
}

export function useRemoveGroupModelResource() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ groupId, providerId, model }: { groupId: string; providerId: string; model: string }) =>
      api.removeGroupModelResource(groupId, providerId, model),
    onSuccess: (_, { groupId }) => invalidateGroupResources(qc, groupId),
  });
}

export function useSetGroupModelResources() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ groupId, resources }: { groupId: string; resources: api.GroupModelResource[] }) =>
      api.setGroupModelResources(groupId, resources),
    onSuccess: (_, { groupId }) => invalidateGroupResources(qc, groupId),
  });
}

export function useGroupDashboard(groupId: string | null, range = "all") {
  return useQuery({
    queryKey: ["group_dashboard", groupId, range],
    queryFn: () => api.getGroupDashboard(groupId!, range),
    enabled: !!groupId,
    refetchInterval: 30_000,
  });
}

export function useRouteTopology() {
  const queryClient = useQueryClient();
  useEffect(() => {
    const unlistenPromise = listen<api.TopologyRuntimeDelta>("topology:runtime-delta", ({ payload }) => {
      queryClient.setQueryData<api.RouteTopology>(["route_topology"], (current) => {
        if (!current || payload.sequence <= current.runtime.sequence) return current;
        const edgeRequests = new Map(payload.edge_deltas.map((edge) => [edge.id, edge.active_requests]));
        return {
          ...current,
          gateway: { ...current.gateway, active_connections: payload.active_connections },
          edges: current.edges.map((edge) => ({
            ...edge,
            active_requests: edgeRequests.get(edge.id) || 0,
            active: (edgeRequests.get(edge.id) || 0) > 0,
          })),
          active_route: payload.latest_route,
          runtime: payload,
          updated_at: payload.emitted_at,
        };
      });
    });
    return () => { void unlistenPromise.then((unlisten) => unlisten()); };
  }, [queryClient]);

  return useQuery({
    queryKey: ["route_topology"],
    queryFn: api.getRouteTopology,
    refetchInterval: 5_000,
  });
}

export function useAgentApps() {
  return useQuery({ queryKey: ["agent_apps"], queryFn: api.detectAgentApps });
}

export function usePreviewAgentAppConfig() {
  return useMutation({
    mutationFn: ({ appId, groupId }: { appId: string; groupId: string }) =>
      api.previewAgentAppConfig(appId, groupId),
  });
}

export function useConfigureAndLaunchAgentApp() {
  return useMutation({
    mutationFn: (params: { appId: string; groupId: string; confirmed: boolean; workingDirectory?: string }) =>
      api.configureAndLaunchAgentApp(params.appId, params.groupId, params.confirmed, params.workingDirectory),
  });
}

export function useRestoreAgentAppConfig() {
  return useMutation({
    mutationFn: ({ snapshotId, force }: { snapshotId: string; force?: boolean }) =>
      api.restoreAgentAppConfig(snapshotId, force),
  });
}

// ============ Logs ============

export function useLogStats(range: api.LogRange = "all") {
  return useQuery({
    queryKey: ["log_stats", range],
    queryFn: () => api.getLogStats(range),
    refetchInterval: 30_000,
  });
}

export function useLogs(
  query: api.LogQuery,
  refetchInterval: number | false = false,
) {
  return useQuery({
    queryKey: ["logs", query.page ?? 1, query.page_size ?? 10, query.range ?? "all", query.status ?? "all", query.keyword ?? "", query.group_id ?? ""],
    queryFn: () => api.queryLogs(query),
    refetchInterval,
  });
}

export function useAnalytics(
  startDate: string,
  endDate: string,
) {
  return useQuery({
    queryKey: ["analytics", startDate, endDate],
    queryFn: () => api.getAnalytics(startDate, endDate),
  });
}

export function useAppLogs(
  params: { page?: number; page_size?: number; keyword?: string } = {},
  refetchInterval: number | false = false,
) {
  return useQuery({
    queryKey: ["app_logs", params],
    queryFn: () => api.readAppLogs(params),
    refetchInterval,
  });
}

export function useAppLogInfo() {
  return useQuery({
    queryKey: ["app_log_info"],
    queryFn: () => api.getAppLogInfo(),
  });
}

// ============ Proxy ============

export function useProxyStatus() {
  return useQuery({
    queryKey: ["proxy_status"],
    queryFn: api.getProxyStatus,
    refetchInterval: 5_000,
  });
}

export function useStartProxy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => api.startProxy(),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["proxy_status"] }),
  });
}

export function useStopProxy() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => api.stopProxy(),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["proxy_status"] }),
  });
}

// ============ Gateway Settings ============

export function useGatewaySettings() {
  return useQuery({
    queryKey: ["gateway_settings"],
    queryFn: api.getGatewaySettings,
  });
}

export function useSetGatewayAccessKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (accessKey: string) => api.setGatewayAccessKey(accessKey),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["gateway_settings"] }),
  });
}

export function useSetCloseButtonBehavior() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (behavior: string) => api.setCloseButtonBehavior(behavior),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["gateway_settings"] }),
  });
}

// ============ Client Keys (virtual keys → route pools) ============

export function useClientKeys() {
  return useQuery({
    queryKey: ["client_keys"],
    queryFn: api.listClientKeys,
  });
}

export function useCreateClientKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: { name: string; poolIds?: string[]; enabled?: boolean }) =>
      api.createClientKey(params.name, params.poolIds, params.enabled),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["client_keys"] }),
  });
}

export function useUpdateClientKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: { id: string; enabled?: boolean; name?: string }) =>
      api.updateClientKey(params.id, { enabled: params.enabled, name: params.name }),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["client_keys"] }),
  });
}

export function useDeleteClientKey() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => api.deleteClientKey(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["client_keys"] }),
  });
}

export function useSetClientKeyPools() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: { clientKeyId: string; poolIds: string[] }) =>
      api.setClientKeyPools(params.clientKeyId, params.poolIds),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["client_keys"] }),
  });
}

// ============ Copilot / Gemini OAuth ============

export function useCopilotPatValidate() {
  return useMutation({
    mutationFn: api.copilotPatValidate,
  });
}

export function useGeminiApiKeyValidate() {
  return useMutation({
    mutationFn: api.geminiApiKeyValidate,
  });
}

// ============ Claude OAuth ============

export function useStartClaudeOAuth() {
  return useMutation({
    mutationFn: (planType: string) => api.startClaudeOAuth(planType),
  });
}

export function useCompleteClaudeOAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ loginId, callbackUrl }: { loginId: string; callbackUrl?: string }) =>
      api.completeClaudeOAuth(loginId, callbackUrl),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useCancelClaudeOAuth() {
  return useMutation({ mutationFn: api.cancelClaudeOAuth });
}

// ============ Copilot Device Flow ============

export function useStartCopilotDeviceFlow() {
  return useMutation({
    mutationFn: api.startCopilotDeviceFlow,
  });
}

export function usePollCopilotDeviceToken() {
  return useMutation({
    mutationFn: ({ deviceCode, intervalMs }: { deviceCode: string; intervalMs: number }) =>
      api.pollCopilotDeviceToken(deviceCode, intervalMs),
  });
}

export function useCompleteCopilotDeviceFlow() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: api.completeCopilotDeviceFlow,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

// ============ Gemini OAuth ============

export function useStartGeminiOAuth() {
  return useMutation({
    mutationFn: api.startGeminiOAuth,
  });
}

export function useCompleteGeminiOAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ loginId, callbackUrl }: { loginId: string; callbackUrl?: string }) =>
      api.completeGeminiOAuth(loginId, callbackUrl),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useCancelGeminiOAuth() {
  return useMutation({ mutationFn: api.cancelGeminiOAuth });
}

// ============ Google Antigravity OAuth ============

export function useStartAntigravityOAuth() {
  return useMutation({
    mutationFn: api.startAntigravityOAuth,
  });
}

export function useCompleteAntigravityOAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ loginId, callbackUrl }: { loginId: string; callbackUrl?: string }) =>
      api.completeAntigravityOAuth(loginId, callbackUrl),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useCancelAntigravityOAuth() {
  return useMutation({ mutationFn: api.cancelAntigravityOAuth });
}

// ============ Grok OAuth ============

export function useStartGrokOAuth() {
  return useMutation({
    mutationFn: api.startGrokOAuth,
  });
}

export function useCompleteGrokOAuth() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ loginId, callbackUrl }: { loginId: string; callbackUrl?: string }) =>
      api.completeGrokOAuth(loginId, callbackUrl),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["accounts"] });
      qc.invalidateQueries({ queryKey: ["providers"] });
    },
  });
}

export function useCancelGrokOAuth() {
  return useMutation({ mutationFn: api.cancelGrokOAuth });
}
