// A method without arguments called without parentheses (IEEE 1800-2017 13.4.4) through an
// element of a queue or an array of handles, as through a handle.
class M;
  string n;
  function new(string x); n = x; endfunction
  virtual function string get_full_name(); return n; endfunction
endclass
module t;
  M maps[$];
  M a[2];
  M h;
  initial begin
    maps.push_back(new("q0"));
    a[1] = new("a1");
    h = new("h");
    if ({maps[0].get_full_name, a[1].get_full_name, h.get_full_name} == "q0a1h") $display("PASSED");
    else $display("FAILED");
  end
endmodule
