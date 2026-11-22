@Component({ selector: "app-root" })
export class AppComponent {
  @Input() title = "";

  ngOnInit(): void {
    this.title = "ready";
  }
}
