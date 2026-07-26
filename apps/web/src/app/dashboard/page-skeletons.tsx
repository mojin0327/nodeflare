export function DashboardPageSkeleton() {
  return (
    <div>
      <div className="flex justify-end mb-6">
        <div className="h-8 w-36 bg-gray-200 rounded-full animate-pulse" />
      </div>
      <div className="flex flex-wrap items-center gap-3 sm:gap-6 mb-6">
        <div className="h-4 w-28 bg-gray-200 rounded animate-pulse" />
        <div className="hidden sm:block h-4 w-px bg-gray-200" />
        <div className="h-4 w-28 bg-gray-200 rounded animate-pulse" />
        <div className="hidden sm:block h-4 w-px bg-gray-200" />
        <div className="h-4 w-20 bg-gray-200 rounded animate-pulse" />
        <div className="ml-auto h-4 w-16 bg-gray-200 rounded animate-pulse" />
      </div>
      <div className="rounded-xl border border-gray-200 overflow-hidden">
        <div className="bg-gray-50 px-4 py-2 border-b border-gray-200 flex items-center justify-between">
          <div className="h-3 w-16 bg-gray-200 rounded animate-pulse" />
          <div className="h-3 w-12 bg-gray-200 rounded animate-pulse" />
        </div>
        <div className="divide-y divide-gray-100">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="flex items-center gap-3 px-4 py-3 bg-white">
              <div className="w-8 h-8 bg-gray-200 rounded-full animate-pulse flex-shrink-0" />
              <div className="flex-1 h-4 bg-gray-200 rounded animate-pulse" />
              <div className="flex items-center gap-3">
                <div className="hidden sm:block h-3 w-12 bg-gray-100 rounded animate-pulse" />
                <div className="h-3 w-16 bg-gray-100 rounded animate-pulse" />
                <div className="w-4 h-4 bg-gray-100 rounded animate-pulse" />
              </div>
            </div>
          ))}
        </div>
      </div>
      <div className="mt-6 rounded-xl border border-gray-200 overflow-hidden">
        <div className="bg-gray-50 px-4 py-2 border-b border-gray-200 flex items-center justify-between">
          <div className="h-3 w-20 bg-gray-200 rounded animate-pulse" />
          <div className="h-3 w-12 bg-gray-200 rounded animate-pulse" />
        </div>
        <div className="flex flex-col sm:flex-row gap-4 sm:gap-8 p-4 bg-white">
          <div className="flex-1 space-y-2">
            <div className="flex items-baseline justify-between">
              <div className="h-3 w-14 bg-gray-200 rounded animate-pulse" />
              <div className="h-3 w-10 bg-gray-200 rounded animate-pulse" />
            </div>
            <div className="h-1.5 bg-gray-100 rounded-full" />
          </div>
          <div className="flex-1 space-y-2">
            <div className="flex items-baseline justify-between">
              <div className="h-3 w-14 bg-gray-200 rounded animate-pulse" />
              <div className="h-3 w-16 bg-gray-200 rounded animate-pulse" />
            </div>
            <div className="h-1.5 bg-gray-100 rounded-full" />
          </div>
        </div>
      </div>
    </div>
  );
}

export function ServersPageSkeleton() {
  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-3">
        <div className="h-8 w-32 bg-gray-100 rounded-lg animate-pulse" />
        <div className="h-7 w-20 bg-gray-200 rounded animate-pulse" />
      </div>
      <div className="grid md:grid-cols-2 lg:grid-cols-3 gap-4">
        {[...Array(6)].map((_, i) => (
          <div key={i} className="rounded-lg border border-gray-200 p-6 animate-pulse">
            <div className="flex items-start justify-between mb-4">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 bg-gray-200 rounded" />
                <div>
                  <div className="h-4 w-28 bg-gray-200 rounded mb-1" />
                  <div className="h-3 w-20 bg-gray-100 rounded" />
                </div>
              </div>
              <div className="h-6 w-20 bg-gray-100 rounded-full" />
            </div>
            <div className="space-y-2">
              <div className="h-3 w-full bg-gray-100 rounded" />
              <div className="h-3 w-3/4 bg-gray-100 rounded" />
            </div>
            <div className="mt-4 pt-4 border-t border-gray-200">
              <div className="h-3 w-full bg-gray-100 rounded" />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function BillingPageSkeleton() {
  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="flex flex-wrap items-center gap-2">
        <div className="h-4 w-20 bg-gray-200 rounded animate-pulse" />
        <div className="h-5 w-16 bg-gray-200 rounded animate-pulse" />
        <div className="h-5 w-12 bg-gray-100 rounded animate-pulse" />
      </div>
      <div className="h-24 bg-gray-100 rounded-xl animate-pulse" />
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 sm:gap-6">
        <div>
          <div className="h-4 w-20 bg-gray-200 rounded animate-pulse mb-2" />
          <div className="border border-gray-300 rounded-lg overflow-hidden">
            <div className="px-4 py-3 text-center border-b border-dashed border-gray-300">
              <div className="h-4 w-20 bg-gray-200 rounded mx-auto mb-1 animate-pulse" />
              <div className="h-3 w-12 bg-gray-100 rounded mx-auto animate-pulse" />
            </div>
            <div className="px-4 py-2 space-y-2 border-b border-dashed border-gray-300">
              <div className="flex justify-between">
                <div className="h-3 w-16 bg-gray-200 rounded animate-pulse" />
                <div className="h-3 w-20 bg-gray-200 rounded animate-pulse" />
              </div>
              <div className="flex justify-between">
                <div className="h-3 w-16 bg-gray-200 rounded animate-pulse" />
                <div className="h-3 w-20 bg-gray-200 rounded animate-pulse" />
              </div>
            </div>
            <div className="px-4 py-2 bg-gray-50">
              <div className="flex justify-between">
                <div className="h-3 w-24 bg-gray-200 rounded animate-pulse" />
                <div className="h-3 w-12 bg-gray-200 rounded animate-pulse" />
              </div>
            </div>
          </div>
        </div>
        <div>
          <div className="h-4 w-20 bg-gray-200 rounded animate-pulse mb-2" />
          <div className="border border-gray-200 rounded-lg overflow-hidden">
            <div className="px-3 py-2 border-b border-gray-200 flex items-center justify-between">
              <div className="w-6 h-6 bg-gray-100 rounded animate-pulse" />
              <div className="h-4 w-24 bg-gray-200 rounded animate-pulse" />
              <div className="w-6 h-6 bg-gray-100 rounded animate-pulse" />
            </div>
            <div className="p-3">
              <div className="grid grid-cols-7 gap-1 mb-1">
                {[...Array(7)].map((_, i) => (
                  <div key={i} className="h-6 bg-gray-100 rounded animate-pulse" />
                ))}
              </div>
              <div className="grid grid-cols-7 gap-1">
                {[...Array(35)].map((_, i) => (
                  <div key={i} className="h-8 bg-gray-50 rounded animate-pulse" />
                ))}
              </div>
            </div>
          </div>
        </div>
      </div>
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4 sm:gap-6">
        <div>
          <div className="h-4 w-24 bg-gray-200 rounded animate-pulse mb-2" />
          <div className="border border-gray-200 rounded-lg p-4 h-[72px] bg-gray-50 animate-pulse" />
        </div>
        <div>
          <div className="h-4 w-20 bg-gray-200 rounded animate-pulse mb-2" />
          <div className="border border-gray-200 rounded-lg p-4 h-[72px] bg-gray-50 animate-pulse" />
        </div>
      </div>
    </div>
  );
}

export function LogsPageSkeleton() {
  return (
    <div className="max-w-6xl">
      <div className="flex items-center justify-between mb-6">
        <div className="h-9 w-40 bg-gray-100 rounded-lg animate-pulse" />
      </div>
      <div className="flex items-center gap-2 mb-4">
        <div className="flex-1 h-9 bg-gray-100 rounded-lg animate-pulse" />
        <div className="h-9 w-20 bg-gray-100 rounded-lg animate-pulse" />
        <div className="h-9 w-9 bg-gray-100 rounded-lg animate-pulse" />
        <div className="h-9 w-9 bg-gray-100 rounded-lg animate-pulse" />
      </div>
      <div className="grid grid-cols-[200px_120px_100px_1fr] gap-2 px-4 py-2 border-b border-gray-200">
        {[...Array(4)].map((_, i) => (
          <div key={i} className="h-3 bg-gray-200 rounded animate-pulse" />
        ))}
      </div>
      <div className="space-y-px py-1">
        {[...Array(12)].map((_, i) => (
          <div key={i} className="grid grid-cols-[200px_120px_100px_1fr] gap-2 px-4 py-2.5">
            <div className="h-4 bg-gray-100 rounded animate-pulse" />
            <div className="h-4 w-16 bg-gray-100 rounded animate-pulse" />
            <div className="h-4 w-12 bg-gray-100 rounded animate-pulse" />
            <div className="h-4 w-32 bg-gray-100 rounded animate-pulse" />
          </div>
        ))}
      </div>
    </div>
  );
}

export function TeamPageSkeleton() {
  return (
    <div>
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 mb-6 sm:mb-8">
        <div />
        <div className="flex items-center gap-2 self-start sm:self-auto">
          <div className="h-8 w-28 bg-gray-100 rounded-lg animate-pulse" />
          <div className="h-7 w-20 bg-gray-200 rounded animate-pulse" />
        </div>
      </div>
      <div className="space-y-3">
        {[...Array(3)].map((_, i) => (
          <div key={i} className="p-4 rounded-xl bg-white border border-gray-100 animate-pulse">
            <div className="flex flex-col sm:flex-row sm:items-center gap-3 sm:gap-4">
              <div className="flex items-center gap-3 sm:gap-4">
                <div className="w-10 h-10 rounded-full bg-gray-200 flex-shrink-0" />
                <div className="flex-1">
                  <div className="h-4 w-32 bg-gray-200 rounded mb-1.5" />
                  <div className="h-3 w-48 bg-gray-100 rounded" />
                </div>
              </div>
              <div className="flex items-center gap-2 ml-[52px] sm:ml-0">
                <div className="h-8 w-24 bg-gray-100 rounded-lg" />
                <div className="h-7 w-12 bg-gray-100 rounded-lg" />
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

export function SettingsPageSkeleton() {
  return (
    <div className="max-w-2xl space-y-8 sm:space-y-10">
      {/* Profile section */}
      <div>
        <div className="h-4 w-24 bg-gray-200 rounded animate-pulse mb-4" />
        <div className="p-4 sm:p-5 rounded-2xl bg-gray-50 border border-gray-100">
          <div className="flex items-center gap-4 sm:gap-5">
            <div className="w-12 h-12 sm:w-16 sm:h-16 rounded-2xl bg-gray-200 animate-pulse flex-shrink-0" />
            <div className="flex-1">
              <div className="h-5 w-36 bg-gray-200 rounded animate-pulse mb-2" />
              <div className="h-4 w-48 bg-gray-100 rounded animate-pulse" />
            </div>
          </div>
        </div>
      </div>
      {/* Notifications section */}
      <div>
        <div className="h-4 w-32 bg-gray-200 rounded animate-pulse mb-4" />
        <div className="space-y-3">
          {[...Array(4)].map((_, i) => (
            <div key={i} className="flex items-center justify-between p-4 rounded-xl bg-white border border-gray-200 animate-pulse">
              <div>
                <div className="h-4 w-40 bg-gray-200 rounded mb-1.5" />
                <div className="h-3 w-56 bg-gray-100 rounded" />
              </div>
              <div className="w-11 h-6 bg-gray-200 rounded-full" />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

export function AuthPageSkeleton() {
  return (
    <div className="max-w-4xl">
      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {[...Array(2)].map((_, i) => (
          <div key={i} className="p-5 bg-white rounded-lg border border-gray-200 animate-pulse min-h-[160px] flex flex-col">
            <div className="flex items-center gap-2 mb-3">
              <div className="w-5 h-5 bg-gray-200 rounded" />
              <div className="h-5 w-32 bg-gray-200 rounded" />
            </div>
            <div className="flex-1 space-y-2">
              <div className="h-3 w-full bg-gray-100 rounded" />
              <div className="h-3 w-4/5 bg-gray-100 rounded" />
            </div>
            <div className="h-4 w-20 bg-gray-100 rounded mt-3" />
          </div>
        ))}
      </div>
    </div>
  );
}

export function VPNPageSkeleton() {
  return (
    <div className="space-y-8">
      <div className="flex items-center justify-end">
        <div className="h-8 w-32 bg-gray-200 rounded animate-pulse" />
      </div>
      <div>
        <div className="grid grid-cols-[1fr_1fr_1fr_2rem] gap-2 pb-2 border-b border-gray-200 mb-1">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="h-3 w-16 bg-gray-200 rounded animate-pulse" />
          ))}
          <div />
        </div>
        <div className="divide-y divide-gray-100">
          {[...Array(3)].map((_, i) => (
            <div key={i} className="grid grid-cols-[1fr_1fr_1fr_2rem] gap-2 py-3">
              <div className="h-4 w-24 bg-gray-200 rounded animate-pulse" />
              <div className="h-4 w-16 bg-gray-100 rounded animate-pulse" />
              <div className="h-4 w-28 bg-gray-100 rounded animate-pulse" />
              <div />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

export function ServerDetailSkeleton() {
  return (
    <div className="max-w-5xl space-y-4 sm:space-y-6">
      <div className="flex flex-col sm:flex-row sm:items-start justify-between gap-4">
        <div className="flex items-center gap-3 sm:gap-4">
          <div className="w-11 h-11 bg-gray-200 rounded-full animate-pulse flex-shrink-0" />
          <div>
            <div className="h-6 w-48 bg-gray-200 rounded animate-pulse mb-1.5" />
            <div className="h-4 w-64 bg-gray-100 rounded animate-pulse" />
          </div>
        </div>
        <div className="flex items-center gap-2 self-start sm:self-auto">
          <div className="h-9 w-36 bg-gray-200 rounded animate-pulse" />
          <div className="w-9 h-9 bg-gray-100 rounded animate-pulse" />
          <div className="w-9 h-9 bg-gray-100 rounded animate-pulse" />
        </div>
      </div>
      <div className="h-10 bg-gray-100 rounded-lg animate-pulse" />
      <div className="flex gap-1 border-b border-gray-200 overflow-x-auto pb-0">
        {[...Array(6)].map((_, i) => (
          <div key={i} className="h-9 w-24 bg-gray-100 rounded-t animate-pulse flex-shrink-0" />
        ))}
      </div>
      <div className="space-y-3">
        {[...Array(4)].map((_, i) => (
          <div key={i} className="h-16 bg-gray-100 rounded-lg animate-pulse" />
        ))}
      </div>
    </div>
  );
}
