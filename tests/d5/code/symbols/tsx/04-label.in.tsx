interface Props {
  label: string;
}

export default function Label({ label }: Props) {
  return <label>{label}</label>;
}
