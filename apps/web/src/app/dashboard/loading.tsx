export default function DashboardLoading() {
  return (
    <div>
      {/* Plan badge - right aligned */}
      <div className="flex justify-end mb-6">
        <div className="h-8 w-36 bg-gray-200 rounded-full animate-pulse" />
      </div>

      {/* Stats row */}
      <div className="flex flex-wrap items-center gap-3 sm:gap-6 mb-6">
        <div className="h-4 w-28 bg-gray-200 rounded animate-pulse" />
        <div className="hidden sm:block h-4 w-px bg-gray-200" />
        <div className="h-4 w-28 bg-gray-200 rounded animate-pulse" />
        <div className="hidden sm:block h-4 w-px bg-gray-200" />
        <div className="h-4 w-20 bg-gray-200 rounded animate-pulse" />
        <div className="ml-auto h-4 w-16 bg-gray-200 rounded animate-pulse" />
      </div>

      {/* Servers table */}
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

      {/* Plan Usage */}
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
