import { useEffect } from "react";
import { useStore } from "./store/useStore";
import { api } from "./api";
import TopBar from "./components/TopBar";
import PodSearch from "./components/PodSearch";
import LogTabs from "./components/LogTabs";
import UnlockGate from "./components/UnlockGate";
import ProfileManagerModal from "./components/ProfileManager";

export default function App() {
  const { storeStatus, bootError, init, managerOpen } = useStore();

  useEffect(() => {
    void init();
  }, [init]);

  // Đóng app khi đang kết nối → ngắt kết nối, không cần confirm (mục 2.3 PLAN)
  useEffect(() => {
    const onUnload = () => {
      void api.disconnect().catch(() => undefined);
    };
    window.addEventListener("beforeunload", onUnload);
    return () => window.removeEventListener("beforeunload", onUnload);
  }, []);

  if (bootError || !storeStatus || storeStatus.locked || !storeStatus.initialized) {
    return <UnlockGate />;
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar />
      <div className="flex min-h-0 flex-1 flex-col">
        <PodSearch />
        <LogTabs />
      </div>
      {managerOpen && <ProfileManagerModal />}
    </div>
  );
}
