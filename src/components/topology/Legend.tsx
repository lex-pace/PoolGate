export default function Legend() {
  return (
    <div className="pg-tv-legend">
      <span><i className="health-healthy" />正常</span>
      <span><i className="health-warning" />告警</span>
      <span><i className="health-fault" />故障</span>
      <span className="pg-tv-legend-rule"><i className="legend-active" />当前路由路径（动态流转）</span>
      <span className="pg-tv-legend-rule"><i className="legend-available" />可用路由关系</span>
    </div>
  );
}
