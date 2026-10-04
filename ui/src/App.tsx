import { useCallback, useEffect, useState } from "react";
import { Database, LineChart, MessageSquare, Settings2 } from "lucide-react";
import { api, type Dataset } from "@/lib/api";
import { Backtest } from "@/screens/Backtest";
import { Chat } from "@/screens/Chat";
import { Data } from "@/screens/Data";
import { Settings } from "@/screens/Settings";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

type Screen = "chat" | "backtest" | "data" | "settings";

export default function App() {
  const [screen, setScreen] = useState<Screen>("chat");
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [version, setVersion] = useState(0);

  const reload = useCallback(() => {
    api.datasets().then(setDatasets).catch(() => setDatasets([]));
    setVersion((v) => v + 1);
  }, []);
  useEffect(reload, [reload]);

  return (
    <Tabs
      value={screen}
      onValueChange={(v) => setScreen(v as Screen)}
      className="flex h-screen flex-col gap-0"
    >
      <header className="flex items-center gap-6 border-b px-4 py-2.5">
        <span className="flex items-center gap-2.5 font-semibold tracking-tight">
          <img src="/quantrig.svg" alt="" width={28} height={28} className="size-7 shrink-0" />
          quantrig
        </span>
        <TabsList>
          <TabsTrigger value="chat">
            <MessageSquare className="size-4" /> Chat
          </TabsTrigger>
          <TabsTrigger value="backtest">
            <LineChart className="size-4" /> Backtest
          </TabsTrigger>
          <TabsTrigger value="data">
            <Database className="size-4" /> Data
          </TabsTrigger>
          <TabsTrigger value="settings">
            <Settings2 className="size-4" /> Settings
          </TabsTrigger>
        </TabsList>
      </header>

      <div className="min-h-0 flex-1 overflow-auto">
        {/* keepMounted: a chat streaming or a strategy half-edited survives a tab switch. */}
        <TabsContent value="chat" className="h-full" keepMounted>
          <Chat onChanged={reload} version={version} />
        </TabsContent>
        <TabsContent value="backtest" className="h-full" keepMounted>
          <Backtest datasets={datasets} onNeedData={() => setScreen("data")} version={version} />
        </TabsContent>
        <TabsContent value="data">
          <Data datasets={datasets} reload={reload} />
        </TabsContent>
        <TabsContent value="settings">
          <Settings onKeySaved={reload} />
        </TabsContent>
      </div>
    </Tabs>
  );
}
