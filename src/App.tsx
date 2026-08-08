// import { Suspense } from "react";
// import { useRoutes } from "react-router-dom";

// import { LoadingScreen } from "@/components/layout/loading-screen";
// import { router } from "./router";
// import { useDesktopServices } from "./hooks/useDesktopServices";

// function App() {
//   useDesktopServices();
//   const routes = useRoutes(router);

//   return (
//     <Suspense fallback={<LoadingScreen label="Loading…" />}>{routes}</Suspense>
//   );
// }

// export default App;

import { Suspense } from "react";
import { useRoutes } from "react-router-dom";

import { LoadingScreen } from "@/components/layout/loading-screen";
import { router } from "./router";
import { useDesktopServices } from "./hooks/useDesktopServices";

function App() {
  const { deviceLimitReached, logoutCountdown } = useDesktopServices();

  const routes = useRoutes(router);

  return (
    <>
      <Suspense fallback={<LoadingScreen label="Loading…" />}>
        {routes}
      </Suspense>

      {deviceLimitReached && (
        <div className="fixed inset-0 z-[99999] flex items-center justify-center bg-black/80">
          <div className="w-full max-w-md rounded-2xl bg-white p-8 text-center shadow-2xl">
            <h2 className="text-2xl font-semibold text-red-600">
              Device Limit Reached
            </h2>

            <p className="mt-4 text-gray-600">
              You have reached the maximum number of devices allowed for your
              current plan.
            </p>

            <p className="mt-2 text-gray-600">
              Please upgrade your plan or remove an existing device before using
              this device.
            </p>

            <div className="mt-6">
              <p className="text-sm text-gray-500">You will be logged out in</p>

              <p className="mt-1 text-4xl font-bold text-red-600">
                {logoutCountdown}
              </p>

              <p className="mt-1 text-sm text-gray-500">seconds</p>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

export default App;
