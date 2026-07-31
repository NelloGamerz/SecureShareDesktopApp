import { Suspense } from "react";
import { useRoutes } from "react-router-dom";

import { LoadingScreen } from "@/components/layout/loading-screen";
import { router } from "./router";
import { useDesktopServices } from "./hooks/useDesktopServices";

function App() {
  useDesktopServices();
  const routes = useRoutes(router);

  return (
    <Suspense fallback={<LoadingScreen label="Loading…" />}>{routes}</Suspense>
  );
}

export default App;
