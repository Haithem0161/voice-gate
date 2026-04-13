import { createRootRouteWithContext, Outlet, Link } from "@tanstack/react-router";
import { TanStackRouterDevtools } from "@tanstack/router-devtools";
import type { QueryClient } from "@tanstack/react-query";

interface RouterContext {
  queryClient: QueryClient;
}

export const Route = createRootRouteWithContext<RouterContext>()({
  component: RootLayout,
});

function RootLayout() {
  return (
    <div className="min-h-screen bg-gray-50">
      <nav className="bg-white shadow-sm border-b border-gray-200">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between h-16 items-center">
            <div className="flex items-center gap-8">
              <Link to="/" className="text-xl font-bold text-gray-900">
                My App
              </Link>
              <div className="flex gap-4">
                <Link
                  to="/"
                  className="text-gray-600 hover:text-gray-900 transition-colors [&.active]:text-blue-600 [&.active]:font-medium"
                >
                  Home
                </Link>
                <Link
                  to="/users"
                  className="text-gray-600 hover:text-gray-900 transition-colors [&.active]:text-blue-600 [&.active]:font-medium"
                >
                  Users
                </Link>
              </div>
            </div>
          </div>
        </div>
      </nav>

      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <Outlet />
      </main>

      {import.meta.env.DEV && <TanStackRouterDevtools />}
    </div>
  );
}
