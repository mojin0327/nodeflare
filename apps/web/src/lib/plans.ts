// Shared plan definitions - must match backend crates/billing/src/plans.rs
//
// Scalability Note: These plan definitions are currently hardcoded for simplicity.
// For dynamic plan updates without redeployment, consider:
// 1. Fetch plans from API: GET /api/v1/plans
// 2. Cache with SWR/React Query for efficient revalidation
// 3. Use environment variables for limits that change frequently
//
// Example dynamic fetch:
// ```typescript
// import useSWR from 'swr';
// const { data: plans } = useSWR('/api/v1/plans', fetcher, { revalidateOnFocus: false });
// ```

export interface PlanLimits {
  max_servers: number;
  max_deployments_per_month: number;
  max_requests_per_month: number;
  max_team_members: number;
  /** Per-server machine memory ceiling (MB). Free=256, paid=2048. */
  max_memory_mb: number;
  log_retention_days: number;
  custom_domains: boolean;
  priority_support: boolean;
  sso_enabled: boolean;
}

/** Allowed per-server memory sizes (MB), smallest first. Mirrors backend MEMORY_LADDER_MB. */
export const MEMORY_LADDER_MB = [256, 512, 1024, 2048] as const;

/** Default size when none is chosen (smallest rung). */
export const DEFAULT_MEMORY_MB = 256;

export interface PlanDefinition {
  plan: 'free' | 'pro' | 'team' | 'enterprise';
  name: string;
  description: string;
  price_monthly_jpy: number | null; // null means "contact us"
  price_yearly_jpy: number | null; // null means "contact us"
  limits: PlanLimits;
  features: string[];
}

// 1 USD = 150 JPY (approximate, rounded for clean pricing)
// Backend: Free $0, Pro $29, Team $99, Enterprise $499
export const PLANS: PlanDefinition[] = [
  {
    plan: 'free',
    name: 'Free',
    description: '個人開発や検証に最適',
    price_monthly_jpy: 0,
    price_yearly_jpy: 0,
    limits: {
      max_servers: 3,
      max_deployments_per_month: 50,
      max_requests_per_month: 10_000,
      max_team_members: 1,
      max_memory_mb: 256,
      log_retention_days: 7,
      custom_domains: false,
      priority_support: false,
      sso_enabled: false,
    },
    features: [
      'サーバー3つまで',
      'デプロイ50回/月',
      '月間10,000リクエスト',
      'ログ7日間保持',
      'コミュニティサポート',
    ],
  },
  {
    plan: 'pro',
    name: 'Pro',
    description: '本番運用に必要な全機能',
    price_monthly_jpy: 2980,
    price_yearly_jpy: 29800, // ~17% off
    limits: {
      max_servers: 20,
      max_deployments_per_month: 500,
      max_requests_per_month: 500_000,
      max_team_members: 1,
      max_memory_mb: 2048,
      log_retention_days: 30,
      custom_domains: true,
      priority_support: false,
      sso_enabled: false,
    },
    features: [
      'サーバー20個まで',
      'デプロイ500回/月',
      '月間500,000リクエスト',
      'ログ30日間保持',
      'カスタムドメイン',
      'メールサポート',
    ],
  },
  {
    plan: 'team',
    name: 'Team',
    description: 'チームでの本格運用に',
    price_monthly_jpy: 9800,
    price_yearly_jpy: 98000, // ~17% off
    limits: {
      max_servers: 100,
      max_deployments_per_month: 2000,
      max_requests_per_month: 5_000_000,
      max_team_members: 10,
      max_memory_mb: 2048,
      log_retention_days: 90,
      custom_domains: true,
      priority_support: true,
      sso_enabled: false,
    },
    features: [
      'サーバー100個まで',
      'デプロイ2000回/月',
      '月間5,000,000リクエスト',
      'ログ90日間保持',
      'カスタムドメイン',
      'チームメンバー10人まで',
      '優先サポート',
    ],
  },
  {
    plan: 'enterprise',
    name: 'Enterprise',
    description: '大規模組織向け',
    price_monthly_jpy: null, // Contact us for pricing
    price_yearly_jpy: null, // Contact us for pricing
    // Scalability: Using very high but finite limits to prevent DDoS/resource exhaustion
    // These are practical limits that no legitimate customer would exceed
    limits: {
      max_servers: 10_000,              // 10K servers max
      max_deployments_per_month: 100_000, // 100K deployments/month
      max_requests_per_month: 1_000_000_000, // 1B requests/month
      max_team_members: 1_000,          // 1K team members
      max_memory_mb: 2048,              // ceiling = ladder max (raise once metered)
      log_retention_days: 365,
      custom_domains: true,
      priority_support: true,
      sso_enabled: true,
    },
    features: [
      'サーバー10,000個まで',
      'デプロイ100,000回/月',
      '月間10億リクエスト',
      'ログ1年間保持',
      'カスタムドメイン',
      'チームメンバー1,000人まで',
      'SSO/SAML',
      '専任サポート',
      'SLA保証',
    ],
  },
];

export function getPlan(planId: string): PlanDefinition | undefined {
  return PLANS.find(p => p.plan === planId);
}

export function findPlanLimits<T extends { plan: string; limits: unknown }>(
  plans: T[] | undefined,
  currentPlan: string | undefined,
): T['limits'] | undefined {
  return plans?.find(p => p.plan === (currentPlan || 'free'))?.limits;
}

export function formatPrice(price: number | null): string {
  if (price === null) return '';
  return `¥${price.toLocaleString()}`;
}
