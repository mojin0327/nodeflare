import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api } from '@/lib/api';

interface McpServerMinimal {
  id: string;
  name: string;
}

export function useServerNameMap(workspaceId: string | undefined): Map<string, string> {
  const { data: servers } = useQuery<McpServerMinimal[]>({
    queryKey: ['servers-minimal', workspaceId],
    queryFn: () => api.get(`/workspaces/${workspaceId}/servers/minimal`),
    enabled: !!workspaceId,
  });

  return useMemo(
    () => new Map(servers?.map(s => [s.id, s.name]) || []),
    [servers],
  );
}
