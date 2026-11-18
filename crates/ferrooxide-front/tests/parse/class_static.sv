class C;
  static int n = 1;
  int id;
  function new();
    n++;
    id = n;
  endfunction
endclass
module m;
  C c;
  initial begin
    C::n = 5;
    c = new;
    $display(c.n, C::n);
  end
endmodule
