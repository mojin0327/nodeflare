export default function DashboardLoading() {
  return (
    <div className="space-y-4">
      <div className="h-5 w-48 bg-gray-200 rounded animate-pulse" />
      <div className="space-y-2">
        {[...Array(5)].map((_, i) => (
          <div key={i} className="h-10 bg-gray-100 rounded-lg animate-pulse" style={{ width: `${85 - i * 5}%` }} />
        ))}
      </div>
      <div className="h-32 bg-gray-100 rounded-xl animate-pulse mt-4" />
    </div>
  );
}
