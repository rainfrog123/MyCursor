import React, { useMemo, useEffect, useState, memo } from "react";
import {
  LineChart,
  Line,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
  ResponsiveContainer,
} from "recharts";
import type { EventsDataCache } from "../types/usage";
import type {
  TooltipProps,
  ChartDataPoint,
  TooltipPayload,
} from "../types/api";
import { createLogger } from "../utils/logger";
import { Icon } from "./Icon";
import { useTheme } from "../context/ThemeContext";

interface EventBasedUsageChartProps {
  email?: string;
  className?: string;
  onTotalCostCalculated?: (totalCost: number) => void; // 回调：将图表计算的总费用传递给父组件
  onModelCostsCalculated?: (modelCosts: Record<string, number>) => void; // 回调：将各模型的费用传递给父组件
}

const logger = createLogger("EventBasedUsageChart");

// 颜色方案：为不同模型分配不同颜色
const MODEL_COLORS = [
  "#3b82f6", // blue
  "#10b981", // green
  "#f59e0b", // amber
  "#8b5cf6", // violet
  "#ec4899", // pink
  "#06b6d4", // cyan
  "#f97316", // orange
  "#6366f1", // indigo
];

// 时间格式（带秒），用于 Tooltip 精确展示
const formatTimeWithSeconds = (timestamp: number): string => {
  const d = new Date(timestamp);
  const mm = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mi = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  return `${mm}-${dd} ${hh}:${mi}:${ss}`;
};

// 自定义Tooltip组件
const CustomTooltip: React.FC<TooltipProps> = ({ active, payload, label }) => {
  if (active && payload && payload.length) {
    // 找到当前数据点
    const dataPoint = payload[0].payload;
    const currentModel = dataPoint.currentModel;
    const currentCost = dataPoint.currentCost;
    const displayTime =
      typeof dataPoint?.timestamp === "number"
        ? formatTimeWithSeconds(dataPoint.timestamp)
        : String(label);

    return (
      <div className="rounded-lg p-3 shadow-lg border" style={{ backgroundColor: 'var(--bg-primary)', borderColor: 'var(--border-primary)', color: 'var(--text-primary)' }}>
        <p className="font-semibold text-gray-700 mb-2">{displayTime}</p>

        {/* 显示当前事件的单次费用 */}
        {currentModel && (
          <div className="mb-2 pb-2 border-b border-gray-200">
            <p className="text-sm text-blue-600 font-medium">
              当前事件: {currentModel}
            </p>
            <p className="text-sm text-blue-800">
              单次费用: ${currentCost?.toFixed(4) || "0.0000"}
            </p>
          </div>
        )}

        {/* 显示所有线的累计费用 */}
        {payload.map((entry: TooltipPayload, index: number) => (
          <div key={`item-${index}`} className="text-sm py-1">
            <span style={{ color: entry.color }} className="font-medium">
              {entry.name}:
            </span>
            <span className="ml-2 text-gray-700">
              ${Number(entry.value).toFixed(4)}
            </span>
          </div>
        ))}
      </div>
    );
  }
  return null;
};

export const EventBasedUsageChart: React.FC<EventBasedUsageChartProps> = memo(({
  email,
  className = "",
  onTotalCostCalculated,
  onModelCostsCalculated,
}) => {
  const [eventsData, setEventsData] = useState<EventsDataCache | null>(null);
  const [loading, setLoading] = useState(true);
  const { config } = useTheme(); // 获取主题配置

  // 加载事件数据
  useEffect(() => {
    const loadEvents = async () => {
      try {
        const { ConfigService } = await import("../services/configService");
        const result = await ConfigService.loadEventsData();

        logger.debug("加载事件数据结果", result);

        if (result.success && result.data) {
          logger.info("事件数据加载成功", {
            events: result.data.events?.length,
            email: result.data.email,
          });

          // 验证数据是否匹配当前邮箱
          if (email && result.data.email === email) {
            setEventsData(result.data);
          } else if (!email) {
            setEventsData(result.data);
          } else {
            logger.warn(`邮箱不匹配: 当前=${email}, 数据=${result.data.email}`);
          }
        } else {
          logger.warn("没有事件数据或加载失败");
        }
      } catch (error) {
        logger.error("加载事件数据失败", error);
      } finally {
        setLoading(false);
      }
    };

    loadEvents();
  }, [email]);

  // 格式化时间戳为可读格式
  const formatTimestamp = (timestamp: number): string => {
    // 检查是否是有效的时间戳
    if (!timestamp || timestamp === 0) {
      logger.warn("无效的时间戳", { timestamp });
      return "Invalid";
    }

    const date = new Date(timestamp);

    // 检查日期是否有效
    if (isNaN(date.getTime())) {
      logger.warn("无法解析的时间戳", { timestamp });
      return "Invalid";
    }

    const month = String(date.getMonth() + 1).padStart(2, "0");
    const day = String(date.getDate()).padStart(2, "0");
    const hours = String(date.getHours()).padStart(2, "0");
    const minutes = String(date.getMinutes()).padStart(2, "0");
    return `${month}-${day} ${hours}:${minutes}`;
  };

  // 处理图表数据：基于事件生成累计费用趋势
  const chartData = useMemo(() => {
    if (!eventsData || !eventsData.events || eventsData.events.length === 0) {
      return { data: [], modelNames: [] };
    }

    const startTime = performance.now();

    // 按时间戳排序事件（使用原地排序避免复制）
    const sortedEvents = eventsData.events
      .filter(e => e.timestamp && e.timestamp > 0 && !isNaN(e.timestamp))
      .sort((a, b) => a.timestamp - b.timestamp);

    // 提取所有模型名称（优化：只遍历一次）
    const modelNames = Array.from(
      new Set(sortedEvents.map(e => e.model_intent).filter(Boolean))
    );

    // 生成累计费用数据点 - 每个事件都生成一个数据点
    const cumulativeCosts: Record<string, number> = {};
    modelNames.forEach((name) => {
      cumulativeCosts[name] = 0;
    });
    let totalCumulativeCost = 0;

    // 生成图表数据点（优化：减少对象创建）
    const data: ChartDataPoint[] = [];
    
    for (let i = 0; i < sortedEvents.length; i++) {
      const event = sortedEvents[i];
      
      // 更新当前事件模型的累计费用和总累计费用
      if (event.model_intent && typeof event.cost_cents === "number") {
        cumulativeCosts[event.model_intent] += event.cost_cents;
        totalCumulativeCost += event.cost_cents;
      }

      // 创建数据点，包含所有模型的当前累计费用
      const dataPoint: ChartDataPoint = {
        timestamp: event.timestamp,
        time: event.timestamp,
        timeLabel: formatTimestamp(event.timestamp),
        totalCost: totalCumulativeCost / 100,
        currentModel: event.model_intent,
        currentCost: event.cost_cents / 100,
      };

      // 添加各模型的累计费用（美元）
      for (const modelName of modelNames) {
        dataPoint[modelName] = cumulativeCosts[modelName] / 100;
      }

      data.push(dataPoint);
    }

    // 如果数据点太多，进行智能采样（优化采样算法）
    let sampledData = data;
    if (data.length > 200) {
      const step = Math.ceil(data.length / 200);
      sampledData = [];
      for (let i = 0; i < data.length; i += step) {
        sampledData.push(data[i]);
      }
      // 确保包含最后一个数据点
      if (sampledData[sampledData.length - 1] !== data[data.length - 1]) {
        sampledData.push(data[data.length - 1]);
      }
    }

    const endTime = performance.now();
    logger.info(`图表数据生成完成，耗时: ${(endTime - startTime).toFixed(2)}ms`, {
      dataPoints: sampledData.length,
      models: modelNames.length,
    });

    // 将费用数据存储在返回值中，稍后在useEffect中调用回调
    return { 
      data: sampledData, 
      modelNames,
      totalCost: totalCumulativeCost,
      modelCosts: cumulativeCosts,
    };
  }, [eventsData]);

  // 使用useEffect在数据计算完成后调用回调（优化：只在数据变化时调用）
  useEffect(() => {
    if (chartData.totalCost !== undefined && onTotalCostCalculated) {
      onTotalCostCalculated(chartData.totalCost);
    }
  }, [chartData.totalCost, onTotalCostCalculated]);

  useEffect(() => {
    if (chartData.modelCosts && onModelCostsCalculated) {
      onModelCostsCalculated(chartData.modelCosts);
    }
  }, [chartData.modelCosts, onModelCostsCalculated]);

  if (loading) {
    return (
      <div className={`p-6 text-center bg-gray-50 rounded-lg ${className}`}>
        <div className="inline-flex items-center">
          <svg className="w-5 h-5 mr-2 animate-spin text-blue-500" viewBox="0 0 24 24">
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
              fill="none"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
            />
          </svg>
          <span className="text-sm text-gray-600">加载事件数据中...</span>
        </div>
      </div>
    );
  }

  if (!eventsData || chartData.data.length === 0) {
    const hasMessage = eventsData?.message;
    const dateRange = eventsData
      ? `${eventsData.start_date.split("T")[0]} \u81f3 ${eventsData.end_date.split("T")[0]}`
      : "未设置";

    return (
      <div
        className={`p-6 text-center bg-gray-50 rounded-lg border-2 border-dashed border-gray-300 ${className}`}
      >
        <div className="max-w-md mx-auto">
          <p className="text-lg font-medium text-gray-700 mb-2">
            📈 {hasMessage ? "该时间范围内暂无使用记录" : "暂无事件明细数据"}
          </p>

          {hasMessage ? (
            <>
              <p className="text-sm text-gray-600 mb-2">
                时间范围: {dateRange}
              </p>
              <div className="bg-yellow-50 border border-yellow-200 rounded-lg p-3 mb-4">
                <p className="text-xs text-yellow-800">
                  💡 {eventsData.message}
                </p>
              </div>
              <p className="text-xs text-gray-500">
                请尝试选择其他时间范围，或在此期间内使用 Cursor 后再刷新。
              </p>
            </>
          ) : (
            <>
              <p className="text-sm text-gray-600 mb-4">
                请点击上方的 "🔄 刷新" 按钮获取用量数据
              </p>
              <div className="bg-blue-50 border border-blue-200 rounded-lg p-3 text-left">
                <p className="text-xs font-medium text-blue-900 mb-1">
                  💡 什么是事件数据？
                </p>
                <p className="text-xs text-blue-700">
                  事件数据记录了每次API调用的详细信息，包括时间、模型、token数量和费用。基于这些数据可以生成更精确的实时费用累计趋势图。
                </p>
              </div>
            </>
          )}
        </div>
      </div>
    );
  }

  // 检测是否开启了自定义背景或透明主题
  const hasCustomBackground = !!(config.customBackground?.enabled && config.customBackground?.imageUrl);
  const isTranslucentTheme = config.mode === 'transparent' || hasCustomBackground;

  return (
    <div
      className={`rounded-lg border p-4 ${className}`}
      style={{
        backgroundColor: 'var(--bg-primary)',
        borderColor: 'var(--border-primary)',
        backdropFilter: isTranslucentTheme ? 'blur(10px)' : 'none',
        WebkitBackdropFilter: isTranslucentTheme ? 'blur(10px)' : 'none',
        transition: 'all 0.3s ease',
      }}
    >
      <div className="flex items-center justify-between mb-4">
        <h4
          className="text-md font-medium flex items-center gap-2"
          style={{
            color: 'var(--text-primary)',
          }}
        >
          <Icon name="trending" size={20} />
          模型使用费用趋势（基于事件数据）
        </h4>
        <div className="flex items-center gap-4">
          <span
            className="text-xs"
            style={{
              color: 'var(--text-secondary)',
            }}
          >
            共 {eventsData.events.length} 个事件
          </span>
          {eventsData.total_events &&
            eventsData.total_events > eventsData.events.length && (
              <span
                className="text-xs"
                style={{
                  color: 'var(--text-secondary)',
                }}
              >
                （总计 {eventsData.total_events} 个，已加载{" "}
                {eventsData.events.length} 个）
              </span>
            )}
        </div>
      </div>
      <ResponsiveContainer width="100%" height={400}>
        <LineChart
          data={chartData.data}
          margin={{ top: 5, right: 30, left: 20, bottom: 5 }}
          style={{
            backgroundColor: 'transparent',
          }}
        >
          <CartesianGrid
            strokeDasharray="3 3"
            stroke={isTranslucentTheme ? 'rgba(0, 0, 0, 0.2)' : undefined}
          />
          <XAxis
            dataKey="time"
            type="number"
            domain={["dataMin", "dataMax"]}
            scale="time"
            tickFormatter={(v) => formatTimestamp(Number(v))}
            allowDuplicatedCategory={false}
            tick={{ fontSize: 12 }}
            angle={-15}
            textAnchor="end"
            height={60}
            padding={{ right: 24 }}
            interval="preserveEnd"
          />
          <YAxis
            tick={{ fontSize: 12 }}
            label={{
              value: "累计费用 ($)",
              angle: -90,
              position: "insideLeft",
            }}
          />
          <Tooltip content={<CustomTooltip />} wrapperStyle={{ zIndex: 1000 }} />
          <Legend wrapperStyle={{ paddingTop: "10px" }} />

          {/* 总累计费用线 */}
          <Line
            type="monotone"
            dataKey="totalCost"
            name="总费用"
            stroke="#000000"
            strokeWidth={3}
            dot={chartData.data.length <= 20}
            activeDot={{ r: 6 }}
            connectNulls={true}
            isAnimationActive={false}
          />

          {/* 各模型的累计费用折线 */}
          {chartData.modelNames.map((modelName, index) => (
            <Line
              key={modelName}
              type="monotone"
              dataKey={modelName}
              name={modelName}
              stroke={MODEL_COLORS[index % MODEL_COLORS.length]}
              strokeWidth={2}
              dot={chartData.data.length <= 20}
              activeDot={{ r: 5 }}
              connectNulls={true}
              isAnimationActive={false}
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
      <div className="mt-2 space-y-1">
        <p
          className="text-xs text-center"
          style={{
            color: 'var(--text-secondary)',
          }}
        >
          *
          黑色粗线显示所有模型的总累计费用，彩色线显示各模型的累计费用，鼠标悬停显示单次调用费用
        </p>
        <p
          className="text-xs text-center"
          style={{
            color: 'var(--text-tertiary)',
          }}
        >
          数据来源: {eventsData.start_date.split("T")[0]} 至{" "}
          {eventsData.end_date.split("T")[0]} | 共 {eventsData.events.length}{" "}
          个事件，显示 {chartData.data.length} 个数据点
        </p>
      </div>
    </div>
  );
});

EventBasedUsageChart.displayName = "EventBasedUsageChart";

// 为了支持 React.lazy 懒加载，添加默认导出
export default EventBasedUsageChart;
