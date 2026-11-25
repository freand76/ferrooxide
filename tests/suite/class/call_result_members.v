// Methods and properties of the object a function returns (`f().m()`, `f().p`, IEEE 1800-2017
// 8.4), also chained.
class R;
  int v = 3;
  function R me(); return this; endfunction
  function int get(); return v; endfunction
endclass
module t;
  R glob = new;
  function R mk(); return glob; endfunction
  initial begin
    if (mk().get() == 3 && mk().me().v == 3 && glob.me().me().get() == 3) $display("PASSED");
    else $display("FAILED");
  end
endmodule
