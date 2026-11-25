class N;
  int m[3];
  static int st[3];
endclass
module t;
  N a[2];
  initial begin
    foreach (N::st[i]) ;
    foreach (a[i].m[j]) ;
    foreach (a[0].m[j]) ;
  end
endmodule
