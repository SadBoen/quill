/**
 * 用 echarts 按需注册图表。
 *
 * **不要 `import echarts from 'echarts'`** —— 那会把 60MB 解包体积里的
 * 全部图表类型拖进产物。按需 `echarts/core` + 具体图表 + 具体组件，
 * 实际入包只有用到的这几件。
 *
 * 新增图表类型时在这里加一行，别在组件里各自 import —— 否则会出现
 * 「某个组件注册了、另一个没注册」的白板。
 */
import * as echarts from 'echarts/core'
import { BarChart, LineChart, PieChart } from 'echarts/charts'
import {
  GridComponent,
  LegendComponent,
  TooltipComponent,
} from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'
import { LabelLayout } from 'echarts/features'

// 只注册真正用到的。少一行就少一份体积 —— 这里每加一个组件，
// 都会进到所有用户的首屏里。
echarts.use([
  BarChart,
  LineChart,
  PieChart,
  GridComponent,
  LegendComponent,
  TooltipComponent,
  LabelLayout,
  CanvasRenderer,
])

export { echarts }
export type EChartsOption = echarts.EChartsCoreOption