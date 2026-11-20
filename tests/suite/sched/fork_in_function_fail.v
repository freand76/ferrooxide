// exit: 1
// A function may only contain fork-join_none (IEEE 1800-2017 13.4.4).
module test;
  function int f(int v);
    fork
      $display("a");
    join_any
    return v;
  endfunction
  initial $display("FAILED %0d", f(1));
endmodule
