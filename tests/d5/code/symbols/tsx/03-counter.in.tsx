export class Counter extends React.Component<Props, State> {
  state = { count: 0 };

  render() {
    return <span>{this.state.count}</span>;
  }
}
