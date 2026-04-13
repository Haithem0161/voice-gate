import { createFileRoute } from "@tanstack/react-router";

export const Route = createFileRoute("/")({
  component: HomePage,
});

function HomePage() {
  return (
    <div className="text-center py-20">
      <h1 className="text-4xl font-bold text-gray-900 mb-4">
        Welcome to My App
      </h1>
      <p className="text-lg text-gray-600 max-w-2xl mx-auto">
        A fullstack application built with React, TanStack, Axum, SQLx, and
        PostgreSQL.
      </p>
    </div>
  );
}
