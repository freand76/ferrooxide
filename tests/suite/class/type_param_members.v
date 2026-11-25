// Static members of the class a type parameter names (IEEE 1800-2017 8.25.1): static
// properties (also inherited), constants and static methods, as calls and as a statement;
// the class comes from another package.
package p;
  class Base; static int cnt = 3; endclass
  class B extends Base;
    localparam int W = 8;
    static string type_name = "B";
    static int calls;
    static function int f(); calls++; return 5; endfunction
  endclass
endpackage
package q;
  class U #(type TYPE = int);
    static function string show();
      TYPE::f();
      return $sformatf("%s %0d %0d %0d %0d", TYPE::type_name, TYPE::f(), TYPE::W, TYPE::cnt, TYPE::calls);
    endfunction
  endclass
endpackage
module t;
  import q::*;
  initial begin
    string s = U#(p::B)::show();
    if (s == "B 5 8 3 2") $display("PASSED"); else $display("FAILED %s", s);
  end
endmodule
