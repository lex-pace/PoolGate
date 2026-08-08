export default function Legend() {
  return (
    <div className="pg-tv-legend">
      <span><i className="health-healthy" />正常</span>
      <span><i className="health-warning" />告警</span>
      <span><i className="health-fault" />故障</span>
      <span><i className="health-disabled" />禁用</span>
      <span><i className="legend-active" />当前路由</span>
      <span><i className="legend-selected" />当前选中</span>
    </div>
  );
}
