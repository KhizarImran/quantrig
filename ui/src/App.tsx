import { useCallback, useEffect, useState } from "react";
import { Database, LineChart, Settings2 } from "lucide-react";
import { api, type Dataset } from "@/lib/api";
import { Backtest } from "@/screens/Backtest";
import { Data } from "@/screens/Data";
import { Settings } from "@/screens/Settings";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

type Screen = "backtest" | "data" | "settings";

export default function App() {
  const [screen, setScreen] = useState<Screen>("backtest");
  const [datasets, setDatasets] = useState<Dataset[]>([]);

  const reload = useCallback(() => {
    api.datasets().then(setDatasets).catch(() => setDatasets([]));
  }, []);
  useEffect(reload, [reload]);

  return (
    <Tabs
      value={screen}
      onValueChange={(v) => setScreen(v as Screen)}
      className="flex h-screen flex-col gap-0"
    >
      <header className="flex items-center gap-6 border-b px-4 py-2.5">
        <span className="font-semibold tracking-tight">quantrig</span>
        <TabsList>
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
        <TabsContent value="backtest" className="h-full">
          <Backtest datasets={datasets} onNeedData={() => setScreen("data")} />
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
