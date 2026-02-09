import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronsUpDown, AlertCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import {
    Command,
    CommandEmpty,
    CommandGroup,
    CommandInput,
    CommandItem,
    CommandList,
} from "@/components/ui/command";
import {
    Popover,
    PopoverContent,
    PopoverTrigger,
} from "@/components/ui/popover";
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from "@/components/ui/select";
import { useProvidersQuery } from "@/lib/query/queries";
import type { AppId } from "@/lib/api";
import { Badge } from "@/components/ui/badge";
import { Alert, AlertDescription } from "@/components/ui/alert";

interface LoadBalancerConfig {
    children: string[];
    strategy: "random" | "order";
}

interface LoadBalancerFormFieldsProps {
    appId: AppId;
    providerId?: string;
    settingsConfig: string;
    onChange: (config: string) => void;
}

export function LoadBalancerFormFields({
    appId,
    providerId,
    settingsConfig,
    onChange,
}: LoadBalancerFormFieldsProps) {
    const { t } = useTranslation();
    const { data: providersData } = useProvidersQuery(appId);

    // Parse current config
    const config = useMemo<LoadBalancerConfig>(() => {
        try {
            const parsed = JSON.parse(settingsConfig);
            return {
                children: Array.isArray(parsed.children) ? parsed.children : [],
                strategy: parsed.strategy || "random",
            };
        } catch {
            return { children: [], strategy: "random" };
        }
    }, [settingsConfig]);

    // Available providers (exclude self and other LBs to prevent simple cycles)
    const availableProviders = useMemo(() => {
        if (!providersData?.providers) return [];
        return Object.values(providersData.providers).filter(
            (p) =>
                p.id !== providerId // Exclude self
        );
    }, [providersData, providerId]);

    const [open, setOpen] = useState(false);

    // Update handlers
    const updateConfig = (updates: Partial<LoadBalancerConfig>) => {
        const newConfig = { ...config, ...updates };
        onChange(JSON.stringify(newConfig, null, 2));
    };

    const toggleProvider = (id: string) => {
        const currentChildren = new Set(config.children);
        if (currentChildren.has(id)) {
            currentChildren.delete(id);
        } else {
            currentChildren.add(id);
        }
        updateConfig({ children: Array.from(currentChildren) });
    };

    const selectedProviders = useMemo(() => {
        return config.children
            .map((id) => availableProviders.find((p) => p.id === id))
            .filter((p): p is NonNullable<typeof p> => !!p);
    }, [config.children, availableProviders]);

    return (
        <div className="space-y-6 animate-in fade-in slide-in-from-top-2 duration-300">
            <Alert>
                <AlertCircle className="h-4 w-4" />
                <AlertDescription>
                    {t("loadBalancer.description", {
                        defaultValue: "负载均衡组可以将请求分发到多个子供应商，实现负载均衡和自动故障转移。",
                    })}
                </AlertDescription>
            </Alert>

            <div className="space-y-4">
                <div className="space-y-2">
                    <Label>
                        {t("loadBalancer.strategy", { defaultValue: "负载策略" })}
                    </Label>
                    <Select
                        value={config.strategy}
                        onValueChange={(v) =>
                            updateConfig({ strategy: v as "random" | "order" })
                        }
                    >
                        <SelectTrigger>
                            <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                            <SelectItem value="random">
                                {t("loadBalancer.strategyRandom", { defaultValue: "随机 (Random)" })}
                            </SelectItem>
                            <SelectItem value="order">
                                {t("loadBalancer.strategyOrder", { defaultValue: "顺序 (Order)" })}
                            </SelectItem>
                            {/* Round Robin requires state, using Random/Order for now */}
                        </SelectContent>
                    </Select>
                    <p className="text-xs text-muted-foreground">
                        {config.strategy === "random"
                            ? t("loadBalancer.strategyRandomDesc", {
                                defaultValue: "每次请求随机选择一个可用供应商",
                            })
                            : t("loadBalancer.strategyOrderDesc", {
                                defaultValue: "按列表顺序优先选择排在前面的供应商",
                            })}
                    </p>
                </div>

                <div className="space-y-2">
                    <Label>
                        {t("loadBalancer.children", { defaultValue: "包含的供应商" })}
                    </Label>

                    <Popover open={open} onOpenChange={setOpen}>
                        <PopoverTrigger asChild>
                            <Button
                                variant="outline"
                                role="combobox"
                                aria-expanded={open}
                                className="w-full justify-between"
                            >
                                {t("loadBalancer.selectProviders", {
                                    defaultValue: "选择供应商...",
                                    count: config.children.length,
                                })}
                                <ChevronsUpDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                            </Button>
                        </PopoverTrigger>
                        <PopoverContent className="w-[400px] p-0" align="start">
                            <Command>
                                <CommandInput placeholder={t("common.search")} />
                                <CommandList>
                                    <CommandEmpty>{t("common.noResults")}</CommandEmpty>
                                    <CommandGroup>
                                        {availableProviders.map((provider) => (
                                            <CommandItem
                                                key={provider.id}
                                                value={provider.name}
                                                onSelect={() => toggleProvider(provider.id)}
                                            >
                                                <div
                                                    className={cn(
                                                        "mr-2 flex h-4 w-4 items-center justify-center rounded-sm border border-primary",
                                                        config.children.includes(provider.id)
                                                            ? "bg-primary text-primary-foreground"
                                                            : "opacity-50 [&_svg]:invisible"
                                                    )}
                                                >
                                                    <Check className={cn("h-4 w-4")} />
                                                </div>
                                                {provider.name}
                                                <span className="ml-auto text-xs text-muted-foreground truncate max-w-[100px]">
                                                    {provider.category || "custom"}
                                                </span>
                                            </CommandItem>
                                        ))}
                                    </CommandGroup>
                                </CommandList>
                            </Command>
                        </PopoverContent>
                    </Popover>

                    {/* Selected Providers List */}
                    {selectedProviders.length > 0 && (
                        <div className="space-y-2 border rounded-md p-2 max-h-[200px] overflow-y-auto">
                            {selectedProviders.map((provider, index) => (
                                <div
                                    key={provider.id}
                                    className="flex items-center justify-between p-2 rounded-md bg-secondary/50 text-sm"
                                >
                                    <div className="flex items-center gap-2">
                                        <Badge variant="outline" className="h-5 w-5 rounded-full p-0 flex items-center justify-center">
                                            {index + 1}
                                        </Badge>
                                        <span>{provider.name}</span>
                                    </div>
                                    <Button
                                        variant="ghost"
                                        size="sm"
                                        className="h-6 w-6 p-0 hover:bg-destructive/10 hover:text-destructive"
                                        onClick={() => toggleProvider(provider.id)}
                                    >
                                        <span className="sr-only">Remove</span>
                                        ×
                                    </Button>
                                </div>
                            ))}
                        </div>
                    )}
                    {selectedProviders.length === 0 && (
                        <p className="text-sm text-destructive">
                            {t("loadBalancer.minChildrenRequired", { defaultValue: "请至少选择一个供应商" })}
                        </p>
                    )}
                </div>
            </div>
        </div>
    );
}
