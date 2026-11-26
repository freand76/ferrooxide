// A class typedef naming a class reaches that class's static methods
// (`C::T::f()`, IEEE 1800-2017 8.23), also when C is declared later
// (`typedef class C;`, 8.27) and T names a specialization of C's own.
module top;
  typedef class later;

  class registry #(type T = int, string N = "");
    static function T create(string name);
      T o = new;
      o.name = {N, ":", name};
      return o;
    endfunction
  endclass

  class early;
    typedef registry #(early, "early") type_id;
    string name;
  endclass

  class user;
    function later get(string n);
      return later::type_id::create(n);
    endfunction
  endclass

  class later;
    typedef registry #(later, "later") type_id;
    string name;
  endclass

  initial begin
    early e;
    later l;
    user u = new;
    e = early::type_id::create("a");
    l = u.get("b");
    if (e.name == "early:a" && l.name == "later:b") $display("PASSED");
    else $display("FAILED: %s %s", e.name, l.name);
  end
endmodule
