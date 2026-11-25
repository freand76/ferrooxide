// A method without arguments called without parentheses (IEEE 1800-2017 13.5.5).
class C;
  int v = 4;
  function int twice(); return 2 * v; endfunction
endclass
module t;
  C c = new;
  initial if (c.twice == 8 && c.twice + 1 == 9) $display("PASSED"); else $display("FAILED");
endmodule
