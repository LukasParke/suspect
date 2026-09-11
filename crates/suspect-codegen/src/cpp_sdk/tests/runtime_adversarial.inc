    // Numeric spellings, signs, Boolean separation and symbolic exponent padding.
    {
        auto number=[](std::string_view value){auto result=JsonNumber::parse(value);CHECK(result);return result.value();};
        CHECK(number("1").compare(number("10"))<0);
        CHECK(number("1.0")==number("1e0"));
        CHECK(number("-1.25e1").compare(number("-10"))<0);
        CHECK(number("1e-999999999999999999999999").compare(number("0"))>0);
        CHECK(number("1e999999999999999999999999").compare(number("9e999999999999999999999998"))>0);
        CHECK(number("-0.0e999999999999999999999999")==number("0"));
        CHECK(number("1000e-000000000000000000003").is_integer());
        CHECK(!number("1000e-000000000000000000004").is_integer());
        auto padded="1e+"+std::string(4000,'0')+"3";
        CHECK(number(padded)==number("1000"));
        auto json=parse_json("["+padded+",true]");CHECK(json);
        CHECK(write_json(json.value()).value()=="["+padded+",true]");
        CHECK(!JsonNumber::parse("true"));CHECK(!JsonNumber::parse("1e"));
        CHECK(!JsonNumber::parse("1e"+std::string(65536,'0')));
    }
    // A single session shares every trial/require budget; later success cannot reset it.
    {
        auto program=detail::validation_program(); program.max_evaluation_steps=1;
        detail::Validation validation(program); bool failed=false;
        try { (void)validation.matches(*program.roots.begin(),JsonValue(),""); }
        catch(const detail::Failure& error){failed=error.error.kind==CodecError::Kind::EvaluationFailure;}
        CHECK(failed);
        program=detail::validation_program(); program.max_equality_steps=0;
        auto constant=std::find_if(program.nodes.begin(),program.nodes.end(),[](const auto& node){return std::any_of(node.checks.begin(),node.checks.end(),[](const auto& c){return c.op==detail::Op::Const;});});
        CHECK(constant!=program.nodes.end());auto root=static_cast<std::size_t>(constant-program.nodes.begin());program.roots.insert(root);
        detail::Validation equal(program);failed=false;
        try{(void)equal.matches(root,JsonValue(),"");}catch(const detail::Failure& error){failed=error.error.kind==CodecError::Kind::EvaluationFailure;}
        CHECK(failed);
    }
    // Equality work remains finite for distinct values, not just duplicates.
    {
        const auto& program=detail::validation_program();
        auto found=std::find_if(program.nodes.begin(),program.nodes.end(),[](const auto& node){return std::any_of(node.checks.begin(),node.checks.end(),[](const auto& c){return c.op==detail::Op::UniqueItems;});});
        CHECK(found!=program.nodes.end());auto root=static_cast<std::size_t>(found-program.nodes.begin());
        JsonValue::Array items;for(int i=0;i<2000;++i)items.emplace_back(JsonNumber(i));
        bool failed=false;detail::Validation validation;
        try{(void)validation.matches(root,JsonValue(std::move(items)),"");}catch(const detail::Failure& error){failed=error.error.kind==CodecError::Kind::EvaluationFailure;}
        CHECK(failed);
    }
