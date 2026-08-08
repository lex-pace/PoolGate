import React, { useState, useEffect, useRef } from 'react';
import { Input } from '@/components/ui/Input';
import { Button } from '@/components/ui/Button';
import { useToast } from '@/components/ui/Toast';
import {
  useCopilotPatValidate,
  useGeminiApiKeyValidate,
  useStartClaudeOAuth,
  useCompleteClaudeOAuth,
  useStartCopilotDeviceFlow,
  usePollCopilotDeviceToken,
  useCompleteCopilotDeviceFlow,
  useStartGeminiOAuth,
  useCompleteGeminiOAuth,
  useStartAntigravityOAuth,
  useCompleteAntigravityOAuth,
  useStartGrokOAuth,
  useCompleteGrokOAuth,
} from '@/hooks/use-tauri';

interface OAuthLoginProps {
  onAccountAdded?: () => void;
}

type ProviderType = 'copilot' | 'claude' | 'gemini' | 'antigravity' | 'grok';
type AuthMode = 'pat' | 'device_flow' | 'oauth_pkce';

const PROVIDER_CONFIG: Record<ProviderType, { label: string; authModes: AuthMode[]; description: string }> = {
  copilot: {
    label: 'GitHub Copilot',
    authModes: ['pat', 'device_flow'],
    description: 'GitHub Copilot Chat API',
  },
  claude: {
    label: 'Claude Code',
    authModes: ['oauth_pkce'],
    description: 'Anthropic Claude 订阅 OAuth (Pro/Max)',
  },
  gemini: {
    label: 'Google Gemini',
    authModes: ['pat', 'oauth_pkce'],
    description: 'Google AI Studio API Key 或 OAuth',
  },
  antigravity: {
    label: 'Google Antigravity',
    authModes: ['oauth_pkce'],
    description: 'Google Antigravity (Cloud Code) 订阅 OAuth',
  },
  grok: {
    label: 'xAI Grok',
    authModes: ['oauth_pkce'],
    description: 'xAI Grok 订阅 OAuth',
  },
};

export function OAuthLogin({ onAccountAdded }: OAuthLoginProps) {
  const { toast } = useToast();
  const [provider, setProvider] = useState<ProviderType>('copilot');
  const [authMode, setAuthMode] = useState<AuthMode>('pat');
  const [credential, setCredential] = useState('');
  const [validationResult, setValidationResult] = useState<string | null>(null);
  
  // Claude OAuth state
  const [claudeLoginId, setClaudeLoginId] = useState<string | null>(null);
  const [claudePlanType, setClaudePlanType] = useState<string>('pro');

  // Copilot Device Flow state
  const [deviceCode, setDeviceCode] = useState<string | null>(null);
  const [userCode, setUserCode] = useState<string | null>(null);
  const [verificationUri, setVerificationUri] = useState<string | null>(null);
  const [pollingActive, setPollingActive] = useState(false);
  const pollingTimer = useRef<ReturnType<typeof setInterval> | null>(null);

  // Gemini/Grok OAuth state
  const [geminiLoginId, setGeminiLoginId] = useState<string | null>(null);
  const [antigravityLoginId, setAntigravityLoginId] = useState<string | null>(null);
  const [grokLoginId, setGrokLoginId] = useState<string | null>(null);

  const copilotPatValidate = useCopilotPatValidate();
  const geminiValidate = useGeminiApiKeyValidate();
  const startClaudeOAuth = useStartClaudeOAuth();
  const completeClaudeOAuth = useCompleteClaudeOAuth();
  const startCopilotDeviceFlow = useStartCopilotDeviceFlow();
  const pollCopilotDeviceToken = usePollCopilotDeviceToken();
  const completeCopilotDeviceFlow = useCompleteCopilotDeviceFlow();
  const startGeminiOAuth = useStartGeminiOAuth();
  const completeGeminiOAuth = useCompleteGeminiOAuth();
  const startAntigravityOAuth = useStartAntigravityOAuth();
  const completeAntigravityOAuth = useCompleteAntigravityOAuth();
  const startGrokOAuth = useStartGrokOAuth();
  const completeGrokOAuth = useCompleteGrokOAuth();

  const config = PROVIDER_CONFIG[provider];
  const loading = copilotPatValidate.isPending || geminiValidate.isPending || startClaudeOAuth.isPending || completeClaudeOAuth.isPending || startCopilotDeviceFlow.isPending || startGeminiOAuth.isPending || completeGeminiOAuth.isPending || startAntigravityOAuth.isPending || completeAntigravityOAuth.isPending || startGrokOAuth.isPending || completeGrokOAuth.isPending;

  // Stop polling on unmount
  useEffect(() => {
    return () => {
      if (pollingTimer.current) {
        clearInterval(pollingTimer.current);
      }
    };
  }, []);

  const handleValidate = async () => {
    if (provider === 'copilot' && authMode === 'pat') {
      if (!credential.trim()) {
        toast('error', '请输入 GitHub PAT');
        return;
      }
      setValidationResult(null);
      try {
        const result = await copilotPatValidate.mutateAsync(credential);
        setValidationResult(result.message);
        toast('success', result.message);
      } catch (error) {
        toast('error', `验证失败: ${error}`);
        setValidationResult(null);
      }
      return;
    }

    if (provider === 'gemini' && authMode === 'pat') {
      if (!credential.trim()) {
        toast('error', '请输入 Google AI Studio API Key');
        return;
      }
      setValidationResult(null);
      try {
        const result = await geminiValidate.mutateAsync(credential);
        setValidationResult(result.message);
        toast('success', result.message);
      } catch (error) {
        toast('error', `验证失败: ${error}`);
        setValidationResult(null);
      }
      return;
    }
  };

  // Copilot Device Flow handlers
  const handleStartDeviceFlow = async () => {
    setValidationResult(null);
    try {
      const result = await startCopilotDeviceFlow.mutateAsync();
      setDeviceCode(result.device_code);
      setUserCode(result.user_code);
      setVerificationUri(result.verification_uri);
      toast('info', '请在浏览器中完成 GitHub 授权');
      startPolling(result.device_code, (result.interval || 5) * 1000);
    } catch (error) {
      toast('error', `启动 Device Flow 失败: ${error}`);
    }
  };

  const startPolling = (code: string, interval: number) => {
    setPollingActive(true);
    pollingTimer.current = setInterval(async () => {
      try {
        const result = await pollCopilotDeviceToken.mutateAsync({
          deviceCode: code,
          intervalMs: 0,
        });
        if (result.authorized && result.access_token) {
          stopPolling();
          try {
            const account = await completeCopilotDeviceFlow.mutateAsync(result.access_token);
            setValidationResult(`Copilot 账号已添加: ${account.name || account.id}`);
            toast('success', 'GitHub Copilot 账号已接入');
            setDeviceCode(null);
            setUserCode(null);
            setVerificationUri(null);
            onAccountAdded?.();
          } catch (error) {
            toast('error', `完成 Copilot 登录失败: ${error}`);
          }
        }
      } catch (error) {
        console.warn('Device flow poll error:', error);
      }
    }, interval);
  };

  const stopPolling = () => {
    if (pollingTimer.current) {
      clearInterval(pollingTimer.current);
      pollingTimer.current = null;
    }
    setPollingActive(false);
  };

  // Claude OAuth handlers
  const handleClaudeOAuthStart = async () => {
    setValidationResult(null);
    try {
      const result = await startClaudeOAuth.mutateAsync(claudePlanType);
      setClaudeLoginId(result.login_id);
      toast('info', '已打开浏览器，请完成 Claude 授权');
      setValidationResult(`授权已启动，请在浏览器中完成登录。Login ID: ${result.login_id}`);
    } catch (error) {
      toast('error', `启动 Claude OAuth 失败: ${error}`);
    }
  };

  const handleClaudeOAuthComplete = async () => {
    if (!claudeLoginId) {
      toast('error', '请先启动 Claude OAuth 授权');
      return;
    }
    setValidationResult(null);
    try {
      const account = await completeClaudeOAuth.mutateAsync({ loginId: claudeLoginId });
      setValidationResult(`Claude 账号已添加: ${account.name || account.id}`);
      toast('success', 'Claude 账号已接入');
      setClaudeLoginId(null);
      onAccountAdded?.();
    } catch (error) {
      toast('error', `Claude OAuth 完成失败: ${error}`);
    }
  };

  // Gemini OAuth handlers
  const handleGeminiOAuthStart = async () => {
    setValidationResult(null);
    try {
      const result = await startGeminiOAuth.mutateAsync();
      setGeminiLoginId(result.login_id);
      toast('info', '已打开浏览器，请完成 Google 授权');
      setValidationResult(`授权已启动，请在浏览器中完成登录。Login ID: ${result.login_id}`);
    } catch (error) {
      toast('error', `启动 Gemini OAuth 失败: ${error}`);
    }
  };

  const handleGeminiOAuthComplete = async () => {
    if (!geminiLoginId) {
      toast('error', '请先启动 Gemini OAuth 授权');
      return;
    }
    setValidationResult(null);
    try {
      const account = await completeGeminiOAuth.mutateAsync({ loginId: geminiLoginId });
      setValidationResult(`Gemini 账号已添加: ${account.name || account.id}`);
      toast('success', 'Gemini 账号已接入');
      setGeminiLoginId(null);
      onAccountAdded?.();
    } catch (error) {
      toast('error', `Gemini OAuth 完成失败: ${error}`);
    }
  };

  // Google Antigravity OAuth handlers
  const handleAntigravityOAuthStart = async () => {
    setValidationResult(null);
    try {
      const result = await startAntigravityOAuth.mutateAsync();
      setAntigravityLoginId(result.login_id);
      toast('info', '已打开浏览器，请完成 Google 授权');
      setValidationResult(`授权已启动，请在浏览器中完成登录。Login ID: ${result.login_id}`);
    } catch (error) {
      toast('error', `启动 Antigravity OAuth 失败: ${error}`);
    }
  };

  const handleAntigravityOAuthComplete = async () => {
    if (!antigravityLoginId) {
      toast('error', '请先启动 Antigravity OAuth 授权');
      return;
    }
    setValidationResult(null);
    try {
      const account = await completeAntigravityOAuth.mutateAsync({ loginId: antigravityLoginId });
      setValidationResult(`Antigravity 账号已添加: ${account.name || account.id}`);
      toast('success', 'Antigravity 账号已接入');
      setAntigravityLoginId(null);
      onAccountAdded?.();
    } catch (error) {
      toast('error', `Antigravity OAuth 完成失败: ${error}`);
    }
  };

  // Grok OAuth handlers
  const handleGrokOAuthStart = async () => {
    setValidationResult(null);
    try {
      const result = await startGrokOAuth.mutateAsync();
      setGrokLoginId(result.login_id);
      toast('info', '已打开浏览器，请完成 xAI 授权');
      setValidationResult(`授权已启动，请在浏览器中完成登录。Login ID: ${result.login_id}`);
    } catch (error) {
      toast('error', `启动 Grok OAuth 失败: ${error}`);
    }
  };

  const handleGrokOAuthComplete = async () => {
    if (!grokLoginId) {
      toast('error', '请先启动 Grok OAuth 授权');
      return;
    }
    setValidationResult(null);
    try {
      const account = await completeGrokOAuth.mutateAsync({ loginId: grokLoginId });
      setValidationResult(`Grok 账号已添加: ${account.name || account.id}`);
      toast('success', 'Grok 账号已接入');
      setGrokLoginId(null);
      onAccountAdded?.();
    } catch (error) {
      toast('error', `Grok OAuth 完成失败: ${error}`);
    }
  };

  const resetState = () => {
    setValidationResult(null);
    setCredential('');
    setClaudeLoginId(null);
    setDeviceCode(null);
    setUserCode(null);
    setVerificationUri(null);
    setGeminiLoginId(null);
    setGrokLoginId(null);
    stopPolling();
  };

  return (
    <div className="pg-card p-4">
      <h3 className="text-sm font-medium mb-3">添加订阅账号</h3>
      
      {/* Provider selector */}
      <div className="flex gap-2 mb-4 flex-wrap">
        {(Object.keys(PROVIDER_CONFIG) as ProviderType[]).map((p) => (
          <button
            key={p}
            className={`px-3 py-1.5 rounded-lg text-sm transition-colors ${
              provider === p
                ? 'bg-blue-500 text-white'
                : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
            }`}
            onClick={() => {
              setProvider(p);
              setAuthMode(PROVIDER_CONFIG[p].authModes[0]);
              resetState();
            }}
          >
            {PROVIDER_CONFIG[p].label}
          </button>
        ))}
      </div>

      {/* Auth mode selector */}
      <div className="flex gap-2 mb-4">
        {config.authModes.map((mode) => (
          <button
            key={mode}
            className={`px-2 py-1 rounded text-xs ${
              authMode === mode
                ? 'bg-blue-100 text-blue-700 border border-blue-300'
                : 'bg-gray-50 text-gray-600 border border-gray-200'
            }`}
            onClick={() => {
              setAuthMode(mode);
              resetState();
            }}
          >
            {mode === 'pat' ? 'PAT' : mode === 'device_flow' ? 'Device Flow' : 'OAuth PKCE'}
          </button>
        ))}
      </div>

      {/* Copilot PAT UI */}
      {provider === 'copilot' && authMode === 'pat' && (
        <>
          <div className="mb-4">
            <label className="text-xs text-gray-500 mb-1 block">
              GitHub Personal Access Token (ghp_...)
            </label>
            <Input
              type="password"
              value={credential}
              onChange={(e) => {
                setCredential(e.target.value);
                setValidationResult(null);
              }}
              placeholder="ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
              disabled={loading}
            />
          </div>
          <Button
            variant="primary"
            onClick={handleValidate}
            disabled={loading || !credential.trim()}
            className="w-full"
          >
            {copilotPatValidate.isPending ? '验证中...' : '验证并添加'}
          </Button>
        </>
      )}

      {/* Copilot Device Flow UI */}
      {provider === 'copilot' && authMode === 'device_flow' && (
        <div>
          {!deviceCode ? (
            <Button
              variant="primary"
              onClick={handleStartDeviceFlow}
              disabled={startCopilotDeviceFlow.isPending}
              className="w-full"
            >
              {startCopilotDeviceFlow.isPending ? '正在获取验证码...' : '开始 Device Flow 授权'}
            </Button>
          ) : (
            <div>
              <div className="p-3 bg-gray-50 border border-gray-200 rounded mb-3">
                <p className="text-sm text-gray-600 mb-2">
                  请在浏览器中访问以下链接并输入验证码：
                </p>
                <a
                  href={verificationUri || 'https://github.com/login/device'}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-blue-600 underline text-sm break-all"
                >
                  {verificationUri || 'https://github.com/login/device'}
                </a>
                <div className="mt-2 p-2 bg-white border border-gray-300 rounded text-center">
                  <span className="text-lg font-mono font-bold text-gray-900">
                    {userCode}
                  </span>
                </div>
              </div>
              {pollingActive && (
                <div className="text-sm text-gray-500 mb-3">
                  等待用户授权中...
                </div>
              )}
              <Button
                variant="outline"
                onClick={() => {
                  stopPolling();
                  resetState();
                }}
                className="w-full"
              >
                取消
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Claude OAuth UI */}
      {provider === 'claude' && (
        <div className="mb-4">
          <div className="flex gap-2 mb-3">
            <button
              className={`px-3 py-1.5 rounded-lg text-sm ${
                claudePlanType === 'pro'
                  ? 'bg-purple-500 text-white'
                  : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
              }`}
              onClick={() => setClaudePlanType('pro')}
            >
              Claude Pro
            </button>
            <button
              className={`px-3 py-1.5 rounded-lg text-sm ${
                claudePlanType === 'max'
                  ? 'bg-purple-500 text-white'
                  : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
              }`}
              onClick={() => setClaudePlanType('max')}
            >
              Claude Max
            </button>
          </div>

          {!claudeLoginId ? (
            <Button
              variant="primary"
              onClick={handleClaudeOAuthStart}
              disabled={loading}
              className="w-full"
            >
              {startClaudeOAuth.isPending ? '正在打开浏览器...' : '开始 Claude 授权'}
            </Button>
          ) : (
            <div>
              <div className="p-2 bg-blue-50 border border-blue-200 rounded text-sm text-blue-700 mb-3">
                请在浏览器中完成 Claude 登录授权，然后点击下方按钮完成接入。
              </div>
              <Button
                variant="primary"
                onClick={handleClaudeOAuthComplete}
                disabled={completeClaudeOAuth.isPending}
                className="w-full"
              >
                {completeClaudeOAuth.isPending ? '正在完成授权...' : '完成 Claude 授权'}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Gemini PAT UI */}
      {provider === 'gemini' && authMode === 'pat' && (
        <>
          <div className="mb-4">
            <label className="text-xs text-gray-500 mb-1 block">
              Google AI Studio API Key (AIza...)
            </label>
            <Input
              type="password"
              value={credential}
              onChange={(e) => {
                setCredential(e.target.value);
                setValidationResult(null);
              }}
              placeholder="AIzaSy..."
              disabled={loading}
            />
          </div>
          <Button
            variant="primary"
            onClick={handleValidate}
            disabled={loading || !credential.trim()}
            className="w-full"
          >
            {geminiValidate.isPending ? '验证中...' : '验证并添加'}
          </Button>
        </>
      )}

      {/* Gemini OAuth UI */}
      {provider === 'gemini' && authMode === 'oauth_pkce' && (
        <div>
          {!geminiLoginId ? (
            <Button
              variant="primary"
              onClick={handleGeminiOAuthStart}
              disabled={startGeminiOAuth.isPending}
              className="w-full"
            >
              {startGeminiOAuth.isPending ? '正在打开浏览器...' : '开始 Gemini 授权'}
            </Button>
          ) : (
            <div>
              <div className="p-2 bg-blue-50 border border-blue-200 rounded text-sm text-blue-700 mb-3">
                请在浏览器中完成 Google 登录授权，然后点击下方按钮完成接入。
              </div>
              <Button
                variant="primary"
                onClick={handleGeminiOAuthComplete}
                disabled={completeGeminiOAuth.isPending}
                className="w-full"
              >
                {completeGeminiOAuth.isPending ? '正在完成授权...' : '完成 Gemini 授权'}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Google Antigravity OAuth UI */}
      {provider === 'antigravity' && (
        <div>
          {!antigravityLoginId ? (
            <Button
              variant="primary"
              onClick={handleAntigravityOAuthStart}
              disabled={startAntigravityOAuth.isPending}
              className="w-full"
            >
              {startAntigravityOAuth.isPending ? '正在打开浏览器...' : '开始 Antigravity 授权'}
            </Button>
          ) : (
            <div>
              <div className="p-2 bg-blue-50 border border-blue-200 rounded text-sm text-blue-700 mb-3">
                请在浏览器中完成 Google 登录授权，然后点击下方按钮完成接入。
              </div>
              <Button
                variant="primary"
                onClick={handleAntigravityOAuthComplete}
                disabled={completeAntigravityOAuth.isPending}
                className="w-full"
              >
                {completeAntigravityOAuth.isPending ? '正在完成授权...' : '完成 Antigravity 授权'}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Grok OAuth UI */}
      {provider === 'grok' && (
        <div>
          {!grokLoginId ? (
            <Button
              variant="primary"
              onClick={handleGrokOAuthStart}
              disabled={startGrokOAuth.isPending}
              className="w-full"
            >
              {startGrokOAuth.isPending ? '正在打开浏览器...' : '开始 Grok 授权'}
            </Button>
          ) : (
            <div>
              <div className="p-2 bg-blue-50 border border-blue-200 rounded text-sm text-blue-700 mb-3">
                请在浏览器中完成 xAI 登录授权，然后点击下方按钮完成接入。
              </div>
              <Button
                variant="primary"
                onClick={handleGrokOAuthComplete}
                disabled={completeGrokOAuth.isPending}
                className="w-full"
              >
                {completeGrokOAuth.isPending ? '正在完成授权...' : '完成 Grok 授权'}
              </Button>
            </div>
          )}
        </div>
      )}

      {/* Validation result */}
      {validationResult && (
        <div className="mt-3 p-2 bg-green-50 border border-green-200 rounded text-sm text-green-700">
          {validationResult}
        </div>
      )}

      {/* Help text */}
      <div className="mt-3 text-xs text-gray-400">
        {config.description}
      </div>
    </div>
  );
}
